//! One ordered photographic definition, shared by complete images and requested prefixes.

use super::*;
use crate::{gpu::GpuInputProgram, pixel_pipeline::PixelPipeline, processing::RollSettings};

#[cfg(all(test, feature = "editor"))]
mod tests;

use crate::gpu::photograph::{Boundary, Component, Step, definition};

#[derive(Clone, Copy, Debug)]
pub(super) enum Interpretation {
    Program(GpuInputProgram, RollSettings),
    Packed(ProcessingParameters),
    Unavailable,
}

impl PartialEq for Interpretation {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Program(a, ar), Self::Program(b, br)) => a == b && ar == br,
            (Self::Packed(a), Self::Packed(b)) => bytemuck::bytes_of(a) == bytemuck::bytes_of(b),
            (Self::Unavailable, Self::Unavailable) => true,
            _ => false,
        }
    }
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
    pub grading: Option<Grading>,
    pub interpretation: Interpretation,
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
            window(image, self.domain.finite()?)?
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
        let interpretation = capture.interpretation;

        Self {
            capture: Some(capture),
            grading: None,
            interpretation,
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
            grading: None,
            interpretation: Interpretation::Program(program, encoding),
            encoded: encoding.exposure != 0.,
            repairs: Default::default(),
            effect_output: None,
        }
    }

    #[cfg(feature = "editor")]
    pub fn filtered(grading: Grading) -> Self {
        Self {
            capture: None,
            grading: Some(grading),
            interpretation: Interpretation::Unavailable,
            encoded: false,
            repairs: Default::default(),
            effect_output: None,
        }
    }

    #[cfg(feature = "editor")]
    pub(super) fn input_program(image: &Arc<ImageNode>) -> Option<crate::gpu::GpuInputProgram> {
        let mut image = Some(image);
        while let Some(node) = image {
            if let Some(native) =
                (node.operation.as_ref() as &dyn std::any::Any).downcast_ref::<Native>()
                && let Interpretation::Program(program, _) = native.interpretation
            {
                return Some(program);
            }
            image = node.input.as_ref();
        }
        None
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
    pub fn preview(
        mut self,
        source: Arc<ImageNode>,
        stages: &PixelPipeline,
        frame: crate::frame_raster::FrameRaster,
        native_grid: Option<crate::frame_raster::FrameRaster>,
    ) -> Result<(Photograph, crate::acquisition::Plan), Error> {
        let layout = crate::pixel_pipeline::sampling::ViewSampling::new(
            frame,
            stages,
            crate::acquisition::Sampling::NeutralFootprints,
        )?;
        let plan = if let Some(grid) = native_grid {
            if grid.composition != layout.presentation.composition
                || grid.min != layout.presentation.min
                || grid.max != layout.presentation.max
            {
                return Err(Error::InvalidParameter(
                    "view grids describe different footprints",
                ));
            }
            if (0..2).any(|axis| grid.output[axis] < layout.plan.output().output[axis]) {
                return Err(Error::InvalidParameter(
                    "acquired view grid is too coarse for processing",
                ));
            }
            crate::acquisition::Plan::new(crate::acquisition::Request {
                output: grid,
                stages,
                sampling: crate::acquisition::Sampling::NeutralFootprints,
            })?
        } else {
            layout.plan
        };
        let native = self.clone().build(
            source.clone(),
            Boundary::Codes,
            Boundary::Neutral,
            &definition(&PixelPipeline::default()),
        )?;
        let neutral = reconstruct(
            native.at(Boundary::Neutral),
            Domain {
                bounds: None,
                grid: Grid {
                    coverage: 8,
                    ..Grid::from_frame(plan.output())
                },
                meaning: Meaning::Neutral,
            },
        )?;
        let presentation = layout.presentation;
        self.grading = self.grading.map(|grading| grading.for_grid(8));
        self.effect_output = Some(EffectOutput {
            domain: Domain {
                bounds: Some(CellRegion::new([0, 0], presentation.output)),
                grid: Grid {
                    coverage: 8,
                    ..Grid::from_frame(presentation)
                },
                meaning: Meaning::Graded,
            },
            offset: [0; 2],
            scale: std::array::from_fn(|i| {
                f64::from(plan.output().output[i]) / f64::from(presentation.output[i])
            }),
        });
        let images = self.build(
            neutral.clone(),
            Boundary::Neutral,
            Boundary::Processed,
            &definition(stages),
        )?;
        Ok((
            Photograph {
                camera: source,
                unrepaired: native.at(Boundary::Unrepaired),
                neutral,
                roll: images.at(Boundary::Roll),
                graded: images.at(Boundary::Graded),
                processed: images.at(Boundary::Processed),
            },
            plan,
        ))
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

    fn camera_image(&self, source: Arc<ImageNode>) -> Arc<ImageNode> {
        ImageNode::new(
            Domain {
                meaning: Meaning::Camera,
                ..source.domain
            },
            Some(source),
            Camera {
                captured: self.capture.as_ref().unwrap().clone(),
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

        for step in steps {
            if !started {
                started = step.boundary == Some(from);
                continue;
            }

            // Encoded readouts cannot cross an arbitrary nonlinear image operation.
            if self.encoded
                && interpreted
                && matches!(&step.component, Component::Effects(effects) if !effects.is_empty())
            {
                return Err(Error::InvalidParameter(
                    "readout encoding cannot cross an added image operation",
                ));
            }

            image = match &step.component {
                Component::Camera => self.camera_image(image),
                Component::Interpretation => ImageNode::new(
                    Domain {
                        meaning: Meaning::NativePositive,
                        ..image.domain
                    },
                    Some(image),
                    Native::new(self.interpretation)?,
                ),
                Component::Working => {
                    let capture = self.capture.as_ref().ok_or(Error::InvalidParameter(
                        "working conversion lacks captured camera metadata",
                    ))?;
                    ImageNode::new(
                        Domain {
                            meaning: Meaning::Neutral,
                            ..image.domain
                        },
                        Some(image),
                        Working::new(capture.interpretation, capture.matrix)?,
                    )
                }
                Component::Repairs => {
                    if !self.repairs.0.is_empty() {
                        let capture = self.capture.as_ref().unwrap();
                        image = Repairs::image(
                            image,
                            self.repairs.clone(),
                            capture.code.clone(),
                            capture.mapping,
                        );
                    }
                    image
                }
                Component::Roll => match &self.grading {
                    Some(grading) => grading.roll(image)?,
                    None => image,
                },
                Component::Layers => match &self.grading {
                    Some(grading) => grading.layers(image)?,
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

fn native_parameters(interpretation: &Interpretation) -> Result<ProcessingParameters, Error> {
    let parameters = interpretation.parameters()?;
    if parameters.graded() {
        Ok(parameters)
    } else {
        ProcessingParameters::digital(Default::default(), glam::DMat3::IDENTITY)
    }
}

struct Native {
    interpretation: Interpretation,
}

impl Native {
    fn new(interpretation: Interpretation) -> Result<Self, Error> {
        native_parameters(&interpretation)?;
        Ok(Self { interpretation })
    }
}

#[derive(Debug, PartialEq)]
struct NativeIdentity(Interpretation);

impl Operation for Native {
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(NativeIdentity(self.interpretation)))
    }

    fn input(&self, region: CellRegion) -> Result<Option<CellRegion>, Error> {
        Ok(Some(region))
    }

    fn pointwise(&self) -> Option<Function> {
        let parameters =
            native_parameters(&self.interpretation).expect("captured interpretation was admitted");
        Some(
            Function::new(
                format!(
                    "{}\n{}",
                    crate::gpu::photograph::pixel_shader(),
                    include_str!("native.wgsl")
                ),
                "native_response",
            )
            .with_uniform(2, &parameters),
        )
    }
}

struct Working {
    interpretation: Interpretation,
    matrix: glam::DMat3,
}

impl Working {
    fn new(interpretation: Interpretation, matrix: glam::DMat3) -> Result<Self, Error> {
        interpretation.parameters()?;
        crate::gpu::CameraMatrix::new(matrix)?;
        Ok(Self {
            interpretation,
            matrix,
        })
    }
}

#[derive(Debug, PartialEq)]
struct WorkingIdentity(Interpretation, glam::DMat3);

impl Operation for Working {
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(WorkingIdentity(
            self.interpretation,
            self.matrix,
        )))
    }

    fn input(&self, region: CellRegion) -> Result<Option<CellRegion>, Error> {
        Ok(Some(region))
    }

    fn pointwise(&self) -> Option<Function> {
        let parameters = self
            .interpretation
            .parameters()
            .expect("captured interpretation was admitted");
        let matrix = crate::gpu::CameraMatrix::new(self.matrix)
            .expect("captured camera matrix was admitted");
        Some(
            Function::new(
                format!(
                    "{}\n{}",
                    crate::gpu::photograph::pixel_shader(),
                    include_str!("working.wgsl")
                ),
                "native_to_working",
            )
            .with_uniform(2, &parameters)
            .with_uniform(3, &matrix),
        )
    }
}
