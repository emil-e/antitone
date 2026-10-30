//! One ordered photographic definition, shared by complete images and requested prefixes.

use super::*;
use crate::{gpu::GpuInputProgram, pixel_pipeline::PixelPipeline, processing::RollSettings};

#[cfg(all(test, feature = "editor"))]
mod tests;

use crate::gpu::photograph::{Boundary, Component, Step, definition};

#[derive(Clone)]
pub(super) enum Interpretation {
    Program(GpuInputProgram, RollSettings),
    Packed(ProcessingParameters),
    Unavailable,
}

impl Interpretation {
    fn parameters(&self) -> Result<ProcessingParameters, Error> {
        match self {
            Self::Program(program, roll) => {
                ProcessingParameters::interpret(program.input, *roll, glam::DMat3::IDENTITY)
            }
            Self::Packed(parameters) => Ok(*parameters),
            Self::Unavailable => Err(Error::InvalidParameter(
                "neutral prefix requires captured input interpretation",
            )),
        }
    }
}

#[derive(Clone)]
pub(super) struct Assembly {
    pub capture: Option<Arc<Captured>>,
    pub photographic: Option<Arc<Photographic>>,
    pub grading: Option<Grading>,
    pub interpretation: Interpretation,
    pub camera_function: Option<Function>,
    pub encoded: bool,
    pub repairs: crate::gpu::repairs::Snapshot,
    #[cfg(feature = "editor")]
    effect_output: Option<EffectOutput>,
}

/// Explicit view-grid transition inside the canonical post-grade effect component.
#[cfg(feature = "editor")]
#[derive(Clone, Copy)]
struct EffectOutput {
    domain: Domain,
    offset: [u32; 2],
    scale: [f64; 2],
}

#[cfg(feature = "editor")]
impl EffectOutput {
    fn compose(
        self,
        effects: &PixelPipeline,
        input: Arc<ImageNode>,
    ) -> Result<Arc<ImageNode>, Error> {
        let split = effects.footprint_suffix();
        let prefix = PixelPipeline {
            stages: effects.stages[..split].to_vec(),
        };
        let suffix = PixelPipeline {
            stages: effects.stages[split..].to_vec(),
        };
        if suffix
            .stages
            .iter()
            .any(|stage| stage.support(self.domain.grid.pitch).0 != [0; 2])
        {
            return Err(Error::InvalidParameter(
                "footprint-integrated effect requires neighbours",
            ));
        }
        let image = prefix.compose(input)?;
        let image = if image.domain.same_grid(self.domain) {
            image
        } else {
            reconstruct_cells(image, self.domain, self.offset.map(f64::from), self.scale)?
        };
        suffix.compose(image)
    }
}

struct Images(Vec<(Boundary, Arc<ImageNode>)>);

impl Images {
    fn at(&self, boundary: Boundary) -> Arc<ImageNode> {
        self.0
            .iter()
            .find(|(at, _)| *at == boundary)
            .unwrap()
            .1
            .clone()
    }
}

impl Assembly {
    pub fn captured(capture: Arc<Captured>) -> Self {
        let interpretation = match &capture.program {
            Some(program) => Interpretation::Program(program.clone(), RollSettings::default()),
            None => capture
                .source_parameters
                .map_or(Interpretation::Unavailable, Interpretation::Packed),
        };

        Self {
            capture: Some(capture),
            photographic: None,
            grading: None,
            interpretation,
            camera_function: None,
            encoded: false,
            repairs: Default::default(),
            #[cfg(feature = "editor")]
            effect_output: None,
        }
    }

    #[cfg(feature = "editor")]
    pub fn native(program: GpuInputProgram, encoding: RollSettings) -> Self {
        Self {
            capture: None,
            photographic: None,
            grading: None,
            interpretation: Interpretation::Program(program, encoding),
            camera_function: None,
            encoded: encoding.exposure != 0.,
            repairs: Default::default(),
            effect_output: None,
        }
    }

    #[cfg(feature = "editor")]
    pub fn filtered(grading: Grading) -> Self {
        Self {
            capture: None,
            photographic: None,
            grading: Some(grading),
            interpretation: Interpretation::Unavailable,
            camera_function: None,
            encoded: false,
            repairs: Default::default(),
            effect_output: None,
        }
    }

    pub fn prefix(
        self,
        source: Arc<ImageNode>,
        from: Boundary,
        to: Boundary,
    ) -> Result<Arc<ImageNode>, Error> {
        Ok(self
            .build(source, from, to, &definition(&PixelPipeline::default()))?
            .at(to))
    }

    pub fn photograph(
        self,
        source: Arc<ImageNode>,
        stages: &PixelPipeline,
    ) -> Result<Photograph, Error> {
        let images = self.build(
            source.clone(),
            Boundary::Codes,
            Boundary::Processed,
            &definition(stages),
        )?;

        Ok(Photograph {
            camera: source,
            unrepaired: images.at(Boundary::Unrepaired),
            neutral: images.at(Boundary::Neutral),
            roll: images.at(Boundary::Roll),
            graded: images.at(Boundary::Graded),
            processed: images.at(Boundary::Processed),
        })
    }

    #[cfg(feature = "editor")]
    pub fn filtered_image(
        mut self,
        source: Arc<ImageNode>,
        stages: &PixelPipeline,
        output: Domain,
        offset: [u32; 2],
        scale: [f64; 2],
    ) -> Result<Arc<ImageNode>, Error> {
        self.effect_output = Some(EffectOutput {
            domain: output,
            offset,
            scale,
        });
        Ok(self
            .build(
                source,
                Boundary::Neutral,
                Boundary::Processed,
                &definition(stages),
            )?
            .at(Boundary::Processed))
    }

    fn camera_image(&self, source: Arc<ImageNode>, output: Output) -> Arc<ImageNode> {
        ImageNode::new(
            Domain {
                meaning: match output {
                    Output::Camera => Meaning::Camera,
                    Output::Neutral => Meaning::Neutral,
                    Output::Roll(_) | Output::Graded(_) => Meaning::Graded,
                },
                ..source.domain
            },
            Some(source),
            Camera {
                captured: self.capture.as_ref().unwrap().clone(),
                output,
            },
        )
    }

    fn build(
        self,
        source: Arc<ImageNode>,
        from: Boundary,
        to: Boundary,
        steps: &[Step],
    ) -> Result<Images, Error> {
        let mut images = Images(vec![(from, source.clone())]);

        if from == to {
            return Ok(images);
        }

        let mut image = source.clone();
        let mut started = from == Boundary::Codes;
        let mut interpreted = from == Boundary::Native;
        // These shortcuts implement only the known camera components. An inserted image
        // operation breaks the segment, so later fused kernels cannot silently skip it.
        let mut folded = started && self.capture.is_some() && self.camera_function.is_none();

        for step in steps {
            if !started {
                started = step.boundary == Some(from);
                continue;
            }

            if !step.component.camera_run() {
                // Encoded light is not an ordinary input to an arbitrary nonlinear
                // operation. Require a component-aware representation rather than
                // applying the operation in the wrong light units.
                if self.encoded && interpreted {
                    return Err(Error::InvalidParameter(
                        "readout encoding cannot cross an added image operation",
                    ));
                }
                folded = false;
            }

            image = match &step.component {
                Component::Camera => match &self.camera_function {
                    Some(function) => pointwise(image, Meaning::Camera, function.clone()),
                    None => self.camera_image(image, Output::Camera),
                },
                Component::Interpretation => ImageNode::new(
                    Domain {
                        meaning: Meaning::NativePositive,
                        ..image.domain
                    },
                    Some(image),
                    Native {
                        parameters: self.interpretation.clone(),
                    },
                ),
                Component::Working => {
                    let capture = self.capture.as_ref().ok_or(Error::InvalidParameter(
                        "working conversion lacks captured camera metadata",
                    ))?;
                    let parameters = capture
                        .source_parameters
                        .unwrap_or_else(ProcessingParameters::source);

                    let function = Function::new(
                        format!(
                            "{}\n{}",
                            crate::gpu::CameraResources::kernel_shader(),
                            include_str!("working.wgsl")
                        ),
                        "native_to_working",
                    )
                    .with_uniform(2, &parameters);
                    let function = if let Some(matrix) = &capture.matrix_buffer {
                        function.with_shared(3, Shared::Buffer(matrix.clone()))
                    } else {
                        function.with_uniform(3, &capture.matrix)
                    };

                    let logical = pointwise(image, Meaning::Neutral, function);

                    if folded {
                        ImageNode::implemented_by(
                            logical,
                            self.camera_image(source.clone(), Output::Neutral),
                        )?
                    } else {
                        logical
                    }
                }
                Component::Repairs => {
                    if !self.repairs.0.is_empty() {
                        let capture = self.capture.as_ref().unwrap();
                        image = ImageNode::new(
                            image.domain,
                            Some(image),
                            Repairs {
                                snapshot: self.repairs.clone(),
                                code: capture.code.clone(),
                                mapping: capture.mapping,
                            },
                        );
                    }
                    image
                }
                Component::Roll => match &self.grading {
                    Some(grading) => {
                        let logical = grading.roll(image)?;

                        if folded {
                            let photo = self.photographic.as_ref().unwrap();
                            ImageNode::implemented_by(
                                logical,
                                self.camera_image(source.clone(), Output::Roll(photo.clone())),
                            )?
                        } else {
                            logical
                        }
                    }
                    None => image,
                },
                Component::Layers => match &self.grading {
                    Some(grading) => {
                        let logical = grading.layers(image)?;

                        if folded {
                            let photo = self.photographic.as_ref().unwrap();
                            ImageNode::implemented_by(
                                logical,
                                self.camera_image(source.clone(), Output::Graded(photo.clone())),
                            )?
                        } else {
                            logical
                        }
                    }
                    None => image,
                },
                Component::Effects(effects) => {
                    #[cfg(feature = "editor")]
                    if let Some(output) = self
                        .effect_output
                        .filter(|_| step.boundary == Some(Boundary::Processed))
                    {
                        image = output.compose(effects, image)?;
                    } else {
                        image = effects.compose(image)?;
                    }
                    #[cfg(not(feature = "editor"))]
                    {
                        image = effects.compose(image)?;
                    }
                    image
                }
            };

            if let Some(boundary) = step.boundary {
                interpreted |= boundary == Boundary::Native;
                images.0.push((boundary, image.clone()));
                if boundary == to {
                    return Ok(images);
                }
            }
        }

        Err(Error::InvalidParameter(
            "photographic endpoint is outside the supplied composition",
        ))
    }
}

struct Native {
    parameters: Interpretation,
}

impl Operation for Native {
    fn input(
        &self,
        region: crate::geometry::SourceRegion,
    ) -> Result<Option<crate::geometry::SourceRegion>, Error> {
        Ok(Some(region))
    }

    fn lower(
        &self,
        fragment: &mut Fragment,
        input: Option<PlannedImage>,
        output: PlannedImage,
    ) -> Result<(), Error> {
        let parameters = self.parameters.parameters()?;

        let parameters = if parameters.graded() {
            parameters
        } else {
            ProcessingParameters::digital(Default::default(), glam::DMat3::IDENTITY)?
        };
        let parameters = fragment.uniform(&parameters);

        fragment.compute(
            format!(
                "{}\n{}",
                crate::gpu::CameraResources::kernel_shader(),
                include_str!("native.wgsl")
            ),
            "native_response",
            vec![
                (2, parameters),
                (20, Binding::Read(input.unwrap().resource)),
                (21, Binding::Write(output.resource)),
            ],
            output.cells(),
        );

        Ok(())
    }
}
