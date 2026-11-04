//! One ordered photographic definition, shared by complete images and requested prefixes.

use super::operations::{Native, Working};
use super::*;
use crate::gpu::grading::Grading;
use crate::gpu::photograph::PhotographGraph;
#[cfg(feature = "editor")]
use crate::processing::RollSettings;
use crate::{gpu::interpretation::InputTransform, pixel_pipeline::PixelPipeline};

#[cfg(all(test, feature = "editor"))]
mod tests;

#[derive(Clone)]
pub struct Assembly {
    pub capture: Option<Arc<Captured>>,
    pub grading: Option<Grading>,
    pub interpretation: Interpretation,
    pub encoded: bool,
    pub repairs: crate::gpu::repairs::Snapshot,
    #[cfg(all(test, feature = "editor"))]
    effect_output: Option<EffectOutput>,
}

/// Explicit view-grid transition inside the canonical post-grade effect component.
#[cfg(all(test, feature = "editor"))]
#[derive(Clone, Copy)]
struct EffectOutput {
    domain: Domain,
    offset: [u32; 2],
    scale: [f64; 2],
}

#[cfg(all(test, feature = "editor"))]
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
        let image = if image.domain().same_grid(self.domain) {
            window(image, self.domain.finite()?)?
        } else {
            reconstruct_cells(image, self.domain, self.offset.map(f64::from), self.scale)?
        };
        suffix.compose(image)
    }
}

pub(crate) struct SourceImages {
    pub codes: Arc<ImageNode>,
    pub camera: Arc<ImageNode>,
    pub camera_positive: Arc<ImageNode>,
    pub unrepaired: Arc<ImageNode>,
}

impl Assembly {
    pub fn photographic(
        limits: &wgpu::Limits,
        definition: CameraInput,
        program: InputTransform,
        grading: Grading,
        repairs: crate::gpu::repairs::Snapshot,
    ) -> Result<Self, Error> {
        let area = grading.area;
        repairs.require_area(area)?;
        admission::prepare(
            limits,
            program.input,
            grading.roll,
            grading.snapshot(),
            program.matrix,
            area,
            repairs.bound(),
        )?;
        let mut assembly = Self::captured(Captured::new(
            definition,
            Interpretation::Program(program, Default::default()),
        )?);
        assembly.grading = Some(grading);
        assembly.repairs = repairs;
        Ok(assembly)
    }

    pub fn captured(capture: Arc<Captured>) -> Self {
        let interpretation = capture.interpretation;

        Self {
            capture: Some(capture),
            grading: None,
            interpretation,
            encoded: false,
            repairs: Default::default(),
            #[cfg(all(test, feature = "editor"))]
            effect_output: None,
        }
    }

    #[cfg(feature = "editor")]
    pub fn native(program: InputTransform, encoding: RollSettings) -> Self {
        Self {
            capture: None,
            grading: None,
            interpretation: Interpretation::Program(program, encoding),
            encoded: encoding.exposure != 0.,
            repairs: Default::default(),
            #[cfg(test)]
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
            #[cfg(test)]
            effect_output: None,
        }
    }

    #[cfg(feature = "editor")]
    pub fn input_transform(
        image: &Arc<ImageNode>,
    ) -> Option<crate::gpu::interpretation::InputTransform> {
        let mut image = Some(image);
        while let Some(node) = image {
            if let Some(native) =
                (node.operation().as_ref() as &dyn std::any::Any).downcast_ref::<Native>()
                && let Interpretation::Program(program, _) = native.interpretation
            {
                return Some(program);
            }
            image = node.input();
        }
        None
    }

    pub fn photograph(
        self,
        source: Arc<ImageNode>,
        stages: &PixelPipeline,
    ) -> Result<PhotographGraph, Error> {
        let images = self.source_images(source)?;
        let neutral = self.repair(images.unrepaired.clone())?;
        let (roll, graded) = self.grade(neutral.clone())?;
        let processed = self.effects(graded.clone(), stages)?;
        Ok(PhotographGraph {
            codes: images.codes,
            camera: images.camera,
            camera_positive: images.camera_positive,
            unrepaired: images.unrepaired,
            neutral,
            roll,
            graded,
            processed,
        })
    }

    #[cfg(all(test, feature = "editor"))]
    pub fn preview(
        mut self,
        source: Arc<ImageNode>,
        stages: &PixelPipeline,
        frame: crate::frame_raster::FrameRaster,
        native_grid: Option<crate::frame_raster::FrameRaster>,
    ) -> Result<(PhotographGraph, crate::acquisition::Plan), Error> {
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
        let native = self.source_images(source.clone())?;
        let repaired = self.repair(native.unrepaired.clone())?;
        let neutral = reconstruct(
            repaired,
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
        let (roll, graded) = self.grade(neutral.clone())?;
        let processed = self.effects(graded.clone(), stages)?;
        Ok((
            PhotographGraph {
                codes: source,
                camera: native.camera,
                camera_positive: native.camera_positive,
                unrepaired: native.unrepaired,
                neutral,
                roll,
                graded,
                processed,
            },
            plan,
        ))
    }

    #[cfg(all(test, feature = "editor"))]
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
        let (_, graded) = self.grade(source)?;
        self.effects(graded, stages)
    }

    pub fn camera(&self, source: Arc<ImageNode>) -> Arc<ImageNode> {
        ImageNode::new(
            Domain {
                meaning: Meaning::Camera,
                ..source.domain()
            },
            Some(source),
            Camera {
                captured: self.capture.as_ref().expect("camera metadata").clone(),
            },
        )
    }

    pub fn interpret(&self, camera: Arc<ImageNode>) -> Result<Arc<ImageNode>, Error> {
        Ok(ImageNode::new(
            Domain {
                meaning: Meaning::NativePositive,
                ..camera.domain()
            },
            Some(camera),
            Native::new(self.interpretation)?,
        ))
    }

    fn working(&self, native: Arc<ImageNode>) -> Result<Arc<ImageNode>, Error> {
        let capture = self.capture.as_ref().ok_or(Error::InvalidParameter(
            "working conversion lacks captured camera metadata",
        ))?;
        Ok(ImageNode::new(
            Domain {
                meaning: match self.interpretation {
                    Interpretation::Packed(parameters) if !parameters.graded() => {
                        Meaning::Inspection
                    }
                    _ => Meaning::Neutral,
                },
                ..native.domain()
            },
            Some(native),
            Working::new(capture.interpretation, capture.matrix)?,
        ))
    }

    pub fn source_images(&self, codes: Arc<ImageNode>) -> Result<SourceImages, Error> {
        let camera = self.camera(codes.clone());
        self.interpret_camera(codes, camera)
    }

    pub fn interpret_camera(
        &self,
        codes: Arc<ImageNode>,
        camera: Arc<ImageNode>,
    ) -> Result<SourceImages, Error> {
        let camera_positive = self.interpret(camera.clone())?;
        let unrepaired = self.working(camera_positive.clone())?;
        Ok(SourceImages {
            codes,
            camera,
            camera_positive,
            unrepaired,
        })
    }

    pub fn unrepaired(&self, codes: Arc<ImageNode>) -> Result<Arc<ImageNode>, Error> {
        Ok(self.source_images(codes)?.unrepaired)
    }

    pub fn repair(&self, image: Arc<ImageNode>) -> Result<Arc<ImageNode>, Error> {
        if self.repairs.0.is_empty() {
            return Ok(image);
        }
        let capture = self.capture.as_ref().ok_or(Error::InvalidParameter(
            "repair conversion lacks captured camera metadata",
        ))?;
        Ok(Repairs::image(
            image,
            self.repairs.clone(),
            capture.code.clone(),
            capture.mapping,
        ))
    }

    pub fn neutral(&self, codes: Arc<ImageNode>) -> Result<Arc<ImageNode>, Error> {
        self.repair(self.unrepaired(codes)?)
    }

    pub fn grade(
        &self,
        neutral: Arc<ImageNode>,
    ) -> Result<(Arc<ImageNode>, Arc<ImageNode>), Error> {
        let Some(grading) = &self.grading else {
            return Ok((neutral.clone(), neutral));
        };
        let roll = grading.roll(neutral)?;
        let graded = grading.layers(roll.clone())?;
        Ok((roll, graded))
    }

    fn effects(
        &self,
        image: Arc<ImageNode>,
        effects: &PixelPipeline,
    ) -> Result<Arc<ImageNode>, Error> {
        if self.encoded && !effects.is_empty() {
            return Err(Error::InvalidParameter(
                "readout encoding cannot cross an added image operation",
            ));
        }
        #[cfg(all(test, feature = "editor"))]
        if let Some(output) = self.effect_output {
            return output.compose(effects, image);
        }
        effects.compose(image)
    }
}
