//! Pure acquisition geometry for ordered finite-neighbourhood pixel stages.
//!
//! The first planner supports symmetric margins on one processing grid. Reconstruction
//! remains a separate sampling boundary: originals are processed before reconstruction,
//! whereas physical/proxy views filter neutral pixels before the photographic grade.
use crate::{
    Error,
    coverage::Coverage,
    frame_raster::FrameRaster,
    geometry::SourceRegion,
    pixel_pipeline::{PixelPipeline, PixelStage},
    processing_support::Support,
};

mod footprints;
mod layout;
mod originals;
pub use originals::{OriginalBatch, OriginalUnit};
pub(crate) mod windows;
pub use footprints::{FilterWindow, FootprintBatch, FootprintBudget, FootprintUnit};
#[cfg(test)]
mod tests;

/// Margins measured in original developed-source cells.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SourceSupport(pub [u32; 2]);

/// Margins measured in cells of the plan's processing raster, at its declared pitch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RasterSupport(pub [u32; 2]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sampling {
    /// Grade/filter original samples, then reconstruct the requested output footprint.
    Originals { reconstruction: SourceSupport },
    /// Filter original neutral AP1 into the processing raster, then grade/filter it.
    NeutralFootprints,
    /// Grade the full camera-cell raster directly; its edges are the real source boundary.
    CameraCells {
        region: SourceRegion,
        size: [u32; 2],
    },
    /// Neutralize an already sampled camera representation, then filter/grade it.
    CameraProxy {
        region: SourceRegion,
        size: [u32; 2],
    },
}

/// Geometric portion of a captured processing request; publication rights stay with the caller.
pub struct Request<'a> {
    pub output: FrameRaster,
    pub stages: &'a PixelPipeline,
    pub sampling: Sampling,
}

/// The source adapter receives this declaration, not photographic stage settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceRead {
    Originals(SourceRegion),
    CameraProxy {
        region: SourceRegion,
        size: [u32; 2],
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProcessingRaster {
    Originals(SourceRegion),
    Frame(FrameRaster),
}

impl ProcessingRaster {
    pub fn size(self) -> [u32; 2] {
        match self {
            Self::Originals(region) => [region.width, region.height],
            Self::Frame(frame) => frame.output,
        }
    }

    pub fn pitch(self) -> [f64; 2] {
        match self {
            Self::Originals(_) => [1.; 2],
            // Composition is a rigid source/frame transform. Derive lengths before
            // transforming; subtracting translated source points can spuriously round
            // an integral support ratio up by another cell.
            Self::Frame(frame) => std::array::from_fn(|i| {
                (frame.max.0[i] - frame.min.0[i]) / f64::from(frame.output[i])
            }),
        }
    }

    fn expand(self, support: RasterSupport, source: SourceRegion) -> Result<Self, Error> {
        match self {
            Self::Originals(region) => Ok(Self::Originals(
                Support(support.0).expand(region, source).validate()?,
            )),
            Self::Frame(frame) => Ok(Self::Frame(Support(support.0).raster(frame)?)),
        }
    }
}

/// Required regions on the stage's own grid, in forward execution order.
#[derive(Clone, Debug, PartialEq)]
pub struct StageRegion {
    pub stage: PixelStage,
    pub support: RasterSupport,
    pub input: ProcessingRaster,
    pub output: ProcessingRaster,
}

/// Mapping from the processing raster to the accepted output, after all pixel stages.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ValidOutput {
    Reconstruction { affine: [[f32; 2]; 3] },
    Extraction { size: [u32; 2], offset: [u32; 2] },
}

/// Immutable geometry and stage intent. Actual source edges use constant outer-cell extension.
#[derive(Clone, Debug)]
pub struct Plan {
    output: FrameRaster,
    coverage: Coverage,
    sampling: Sampling,
    pipeline: PixelPipeline,
    stages: Vec<StageRegion>,
    input: ProcessingRaster,
    pitch: [f64; 2],
    read: SourceRead,
    source_support: SourceSupport,
    valid_output: ValidOutput,
}

impl Plan {
    pub fn new(request: Request<'_>) -> Result<Self, Error> {
        let Request {
            output,
            stages: pipeline,
            sampling,
        } = request;
        let source = output.composition.area().validate()?;
        if output.output.contains(&0)
            || (0..2).any(|i| {
                !output.min.0[i].is_finite()
                    || !output.max.0[i].is_finite()
                    || output.min.0[i] >= output.max.0[i]
            })
        {
            return Err(Error::InvalidParameter("invalid acquisition raster"));
        }
        let mut required = match sampling {
            Sampling::Originals { reconstruction } => ProcessingRaster::Originals(
                Support(reconstruction.0)
                    .expand(output.processing_bounds()?, source)
                    .validate()?,
            ),
            Sampling::NeutralFootprints
            | Sampling::CameraProxy { .. }
            | Sampling::CameraCells { .. } => ProcessingRaster::Frame(output),
        };
        let pitch = required.pitch();
        if pitch.iter().any(|p| !p.is_finite() || *p <= 0.) {
            return Err(Error::InvalidParameter("invalid acquisition pitch"));
        }
        let mut stages = Vec::with_capacity(pipeline.stages.len());
        let mut margin = Support::NONE;
        for stage in pipeline.stages.iter().rev().cloned() {
            let support = RasterSupport(stage.support(pitch).0);
            let input = if matches!(sampling, Sampling::CameraCells { .. }) {
                // The entire real-source cell grid is present; kernels extend its edge cells.
                required
            } else {
                required.expand(support, source)?
            };
            stages.push(StageRegion {
                stage,
                support,
                input,
                output: required,
            });
            margin = margin.then(Support(support.0))?;
            required = input;
        }
        stages.reverse();
        let read = match sampling {
            Sampling::Originals { .. } | Sampling::NeutralFootprints => {
                SourceRead::Originals(match required {
                    ProcessingRaster::Originals(region) => region,
                    ProcessingRaster::Frame(frame) => frame.processing_bounds()?,
                })
            }
            Sampling::CameraProxy { region, size } | Sampling::CameraCells { region, size } => {
                region.validate()?;
                if size.contains(&0) || region != source {
                    return Err(Error::InvalidParameter(
                        "proxy does not represent the declared source",
                    ));
                }
                if matches!(sampling, Sampling::CameraCells { .. }) {
                    let composition = crate::geometry::Composition::new(
                        source,
                        Default::default(),
                        Default::default(),
                    )?;
                    let full = FrameRaster {
                        output: size,
                        ..FrameRaster::full(composition)?
                    };
                    if output != full {
                        return Err(Error::InvalidParameter(
                            "direct camera cells require the full source grid",
                        ));
                    }
                }
                SourceRead::CameraProxy { region, size }
            }
        };
        let valid_output = match required {
            ProcessingRaster::Originals(bounds) => ValidOutput::Reconstruction {
                affine: output.affine_in(bounds)?,
            },
            ProcessingRaster::Frame(_) => ValidOutput::Extraction {
                size: output.output,
                offset: if matches!(sampling, Sampling::CameraCells { .. }) {
                    [0; 2]
                } else {
                    margin.0
                },
            },
        };
        // Native stage support is used for repair influence before output reconstruction.
        // The reconstruction roundoff guard is not a photographic neighbourhood.
        let source_support = match sampling {
            Sampling::Originals { .. } => SourceSupport(margin.0),
            _ => SourceSupport([0; 2]),
        };
        Ok(Self {
            output,
            coverage: output.composition.coverage(output.min, output.max)?,
            sampling,
            pipeline: pipeline.clone(),
            stages,
            input: required,
            pitch,
            read,
            source_support,
            valid_output,
        })
    }

    pub fn output(&self) -> FrameRaster {
        self.output
    }
    pub fn sampling(&self) -> Sampling {
        self.sampling
    }
    pub fn pipeline(&self) -> &PixelPipeline {
        &self.pipeline
    }
    pub fn stages(&self) -> &[StageRegion] {
        &self.stages
    }
    pub fn input(&self) -> ProcessingRaster {
        self.input
    }
    /// Source units per processing cell, preserved through padding and input reuse.
    pub fn pitch(&self) -> [f64; 2] {
        self.pitch
    }

    pub fn source_read(&self) -> SourceRead {
        self.read
    }

    /// Raster extraction offset for filter-before-grade views; originals use affine reconstruction.
    pub fn output_offset(&self) -> Option<[u32; 2]> {
        match self.valid_output {
            ValidOutput::Extraction { offset, .. } => Some(offset),
            ValidOutput::Reconstruction { .. } => None,
        }
    }

    pub fn valid_output(&self) -> ValidOutput {
        self.valid_output
    }

    /// Original-cell repair relevance for process-before-reconstruction execution.
    pub fn original_repair_contributes(&self, point: [u32; 2]) -> Result<bool, Error> {
        if !matches!(self.sampling, Sampling::Originals { .. }) {
            return Err(Error::InvalidParameter(
                "original repair relevance requires original sampling",
            ));
        }
        let area = self.output.composition.area();
        if point[0] < area.x
            || point[1] < area.y
            || point[0] >= area.x + area.width
            || point[1] >= area.y + area.height
        {
            return Ok(false);
        }
        let affected =
            Support(self.source_support.0).expand([point[0], point[1], 1, 1].into(), area);
        let mut low = [affected.x, affected.y].map(f64::from);
        let mut high = [affected.x + affected.width, affected.y + affected.height].map(f64::from);
        let source_low = [area.x, area.y].map(f64::from);
        let source_high = [area.x + area.width, area.y + area.height].map(f64::from);
        // Actual source edge cells extend into exterior processing windows. A clipped
        // bounding box alone would omit repairs influencing those extended cells.
        for point in self.coverage.corners() {
            for axis in 0..2 {
                if low[axis] <= source_low[axis] {
                    low[axis] = low[axis].min(point.0[axis]);
                }
                if high[axis] >= source_high[axis] {
                    high[axis] = high[axis].max(point.0[axis]);
                }
            }
        }
        Ok(self
            .coverage
            .rectangle_weight(low, std::array::from_fn(|i| high[i] - low[i]))
            > 0.)
    }

    pub fn output_coverage(&self) -> Coverage {
        self.coverage
    }
}
