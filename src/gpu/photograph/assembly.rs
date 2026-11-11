//! One ordered photographic definition, shared by complete images and requested prefixes.

use super::operations::{Native, Working};
use super::*;
use crate::gpu::grading::Grading;
use crate::gpu::photograph::PhotographGraph;
use crate::{gpu::interpretation::InputTransform, pixel_pipeline::PixelPipeline};

#[cfg(all(test, feature = "editor"))]
mod tests;

#[derive(Clone)]
pub struct Assembly {
    pub capture: Option<Arc<Captured>>,
    pub grading: Option<Grading>,
    pub interpretation: Interpretation,
    pub repairs: Vec<crate::dust_repair::Repair>,
    #[cfg(test)]
    pub replacements: Vec<crate::gpu::repairs::fixtures::Replacement>,
}

/// View-grid transition between spatial and footprint-integrated effects.
#[derive(Clone, Copy)]
pub(super) struct EffectOutput {
    domain: Domain,
    offset: [u32; 2],
    scale: [f64; 2],
}

impl EffectOutput {
    #[cfg(feature = "editor")]
    pub(super) fn new(
        presentation: crate::frame_raster::FrameRaster,
        processing: crate::frame_raster::FrameRaster,
        coverage: u32,
    ) -> Self {
        Self {
            domain: view_domain(presentation, coverage),
            offset: [0; 2],
            scale: std::array::from_fn(|i| {
                f64::from(processing.output[i]) / f64::from(presentation.output[i])
            }),
        }
    }

    fn compose(self, effects: &PixelPipeline, mut image: Value) -> Result<Value, Error> {
        let split = effects.footprint_suffix();
        for stage in &effects.stages[..split] {
            image = stage.compose(image)?;
        }
        image = if image.domain().same_grid(self.domain) {
            window(image, self.domain.finite()?)?
        } else {
            reconstruct_cells(image, self.domain, self.offset.map(f64::from), self.scale)?
        };
        for stage in &effects.stages[split..] {
            image = stage.compose(image)?;
        }
        Ok(image)
    }
}

pub(super) struct Finished {
    neutral: Value,
    roll: Value,
    graded: Value,
    pub(super) processed: Value,
}

impl Finished {
    pub(super) fn with_source(self, source: SourceImages) -> PhotographGraph {
        PhotographGraph {
            codes: source.codes,
            camera: source.camera,
            camera_positive: source.camera_positive,
            unrepaired: source.unrepaired,
            neutral: self.neutral,
            roll: self.roll,
            graded: self.graded,
            processed: self.processed,
        }
    }
}

pub(crate) struct SourceImages {
    pub codes: Value,
    pub camera: Value,
    pub camera_positive: Value,
    pub unrepaired: Value,
}

impl Assembly {
    pub fn photographic(
        limits: &wgpu::Limits,
        definition: CameraInput,
        program: InputTransform,
        grading: Grading,
        repairs: Vec<crate::dust_repair::Repair>,
    ) -> Result<Self, Error> {
        let area = grading.area;
        admission::prepare(
            limits,
            program.input,
            grading.roll,
            grading.snapshot(),
            program.matrix,
            area,
        )?;
        let mut assembly =
            Self::captured(Captured::new(definition, Interpretation::Program(program))?);
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
            repairs: Default::default(),
            #[cfg(test)]
            replacements: Vec::new(),
        }
    }

    #[cfg(feature = "editor")]
    pub fn native(program: InputTransform) -> Self {
        Self {
            capture: None,
            grading: None,
            interpretation: Interpretation::Program(program),
            repairs: Default::default(),
            #[cfg(test)]
            replacements: Vec::new(),
        }
    }

    #[cfg(feature = "editor")]
    pub fn filtered(grading: Grading) -> Self {
        Self {
            capture: None,
            grading: Some(grading),
            interpretation: Interpretation::Unavailable,
            repairs: Default::default(),
            #[cfg(test)]
            replacements: Vec::new(),
        }
    }

    #[cfg(feature = "editor")]
    pub fn input_transform(
        image: &Value,
    ) -> Result<Option<crate::gpu::interpretation::InputTransform>, Error> {
        let mut captured = None;
        for node in image.ancestors() {
            if let Some(native) = node.parameters::<super::operations::NativeIdentity>()
                && let Interpretation::Program(program) = native.0
            {
                if captured.is_some_and(|prior| prior != program) {
                    return Err(Error::InvalidParameter(
                        "calculation mixes source interpretations",
                    ));
                }
                captured = Some(program);
            }
        }
        Ok(captured)
    }

    pub fn photograph(
        self,
        source: Value,
        stages: &PixelPipeline,
    ) -> Result<PhotographGraph, Error> {
        let images = self.source_images(source)?;
        let neutral = self.repair(images.unrepaired.clone())?;
        Ok(self.finish(neutral, stages, None)?.with_source(images))
    }

    pub(super) fn finish(
        &self,
        neutral: Value,
        stages: &PixelPipeline,
        output: Option<EffectOutput>,
    ) -> Result<Finished, Error> {
        let (roll, graded) = self.grade(neutral.clone())?;
        let processed = Self::effects(graded.clone(), stages, output)?;
        Ok(Finished {
            neutral,
            roll,
            graded,
            processed,
        })
    }

    #[cfg(all(test, feature = "editor"))]
    pub fn preview(
        mut self,
        source: Value,
        stages: &PixelPipeline,
        frame: crate::frame_raster::FrameRaster,
        native_grid: Option<crate::frame_raster::FrameRaster>,
    ) -> Result<(PhotographGraph, crate::acquisition::Plan), Error> {
        let layout = crate::pixel_pipeline::sampling::ViewSampling::new(
            frame,
            stages,
            crate::acquisition::Sampling::NeutralFootprints,
        )?
        .with_native_grid(native_grid)?;
        let plan = layout.plan;
        let native = self.source_images(source)?;
        let repaired = self.repair(native.unrepaired.clone())?;
        let neutral = reconstruct_tiled(
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
        self.grading = self.grading.map(|grading| grading.for_grid(8));
        let output = EffectOutput::new(layout.presentation, plan.output(), 8);
        Ok((
            self.finish(neutral, stages, Some(output))?
                .with_source(native),
            plan,
        ))
    }

    #[cfg(all(test, feature = "editor"))]
    pub fn filtered_image(
        self,
        source: Value,
        stages: &PixelPipeline,
        output: Domain,
        offset: [u32; 2],
        scale: [f64; 2],
    ) -> Result<Value, Error> {
        let (_, graded) = self.grade(source)?;
        Self::effects(
            graded,
            stages,
            Some(EffectOutput {
                domain: output,
                offset,
                scale,
            }),
        )
    }

    pub fn camera(&self, source: Value) -> Value {
        Value::from_operation(
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

    pub fn interpret(&self, camera: Value) -> Result<Value, Error> {
        Ok(Value::from_operation(
            Domain {
                meaning: Meaning::NativePositive,
                ..camera.domain()
            },
            Some(camera),
            Native::new(self.interpretation)?,
        ))
    }

    fn working(&self, native: Value) -> Result<Value, Error> {
        let capture = self.capture.as_ref().ok_or(Error::InvalidParameter(
            "working conversion lacks captured camera metadata",
        ))?;
        Ok(Value::from_operation(
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

    pub fn source_images(&self, codes: Value) -> Result<SourceImages, Error> {
        let camera = self.camera(codes.clone());
        self.interpret_camera(codes, camera)
    }

    pub fn interpret_camera(&self, codes: Value, camera: Value) -> Result<SourceImages, Error> {
        let camera_positive = self.interpret(camera.clone())?;
        let unrepaired = self.working(camera_positive.clone())?;
        Ok(SourceImages {
            codes,
            camera,
            camera_positive,
            unrepaired,
        })
    }

    pub fn unrepaired(&self, codes: Value) -> Result<Value, Error> {
        Ok(self.source_images(codes)?.unrepaired)
    }

    pub fn repair(&self, image: Value) -> Result<Value, Error> {
        let needed = !self.repairs.is_empty();
        #[cfg(test)]
        let needed = needed || !self.replacements.is_empty();
        if !needed {
            return Ok(image);
        }
        let domain = image.domain();
        // Resident original-cell arguments have local indices; masks name absolute source cells.
        let image = if domain.grid != Grid::axis_aligned([0.; 2], [1.; 2]) {
            let Some(Mapping::Resident(Some(geometry))) =
                self.capture.as_ref().map(|capture| capture.mapping)
            else {
                return Err(Error::InvalidParameter(
                    "healing requires original neutral cells",
                ));
            };
            let region = geometry.region();
            if geometry.is_pair()
                || geometry.size() != [region.width, region.height]
                || domain.grid.axes != [[1., 0.], [0., 1.]]
                || domain.grid.origin != [f64::from(region.x), f64::from(region.y)]
            {
                return Err(Error::InvalidParameter(
                    "healing requires original neutral cells",
                ));
            }
            crate::gpu::evaluation::reconstruct(
                image,
                Domain {
                    bounds: Some(region.try_into()?),
                    grid: Grid::axis_aligned([0.; 2], [1.; 2]),
                    meaning: domain.meaning,
                },
            )?
        } else {
            image
        };
        let image = if self.repairs.is_empty() {
            image
        } else {
            crate::gpu::repairs::definition::compose(image, &self.repairs)?
        };
        #[cfg(test)]
        let image = if self.replacements.is_empty() {
            image
        } else {
            crate::gpu::repairs::definition::compose_images(image, self.replacements.clone())?
        };
        if image.domain() == domain {
            Ok(image)
        } else {
            crate::gpu::evaluation::reconstruct(image, domain)
        }
    }

    pub fn neutral(&self, codes: Value) -> Result<Value, Error> {
        self.repair(self.unrepaired(codes)?)
    }

    pub fn grade(&self, neutral: Value) -> Result<(Value, Value), Error> {
        let Some(grading) = &self.grading else {
            return Ok((neutral.clone(), neutral));
        };
        let roll = grading.roll(neutral)?;
        let graded = grading.layers(roll.clone())?;
        Ok((roll, graded))
    }

    fn effects(
        image: Value,
        effects: &PixelPipeline,
        output: Option<EffectOutput>,
    ) -> Result<Value, Error> {
        match output {
            Some(output) => output.compose(effects, image),
            None => effects.compose(image),
        }
    }
}
