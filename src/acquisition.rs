//! Acquisition layouts on original or frame grids. Frame dependencies follow composed nodes.

use crate::{
    Error, frame_raster::FrameRaster, geometry::SourceRegion, pixel_pipeline::PixelPipeline,
    processing_support::Support,
};

mod footprints;
mod graph;

mod layout;

mod originals;

pub use originals::{OriginalBatch, OriginalUnit};

pub(crate) mod windows;

pub use footprints::WindowBudget;

#[cfg(test)]
mod tests;
#[cfg(test)]
use crate::pixel_pipeline::PixelStage;

/// Margins measured in original developed-source cells.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SourceSupport(pub [u32; 2]);

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
    sampling: Sampling,
    pipeline: PixelPipeline,
    input: ProcessingRaster,
    pitch: [f64; 2],
    read: SourceRead,
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
        let required = match sampling {
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
        let (required, valid_output) = match required {
            ProcessingRaster::Originals(region) => {
                let region = graph::original_input(source, region, pipeline)?;
                (
                    ProcessingRaster::Originals(region),
                    ValidOutput::Reconstruction {
                        affine: output.affine_in(region)?,
                    },
                )
            }
            ProcessingRaster::Frame(_) => {
                let (input, offset) = graph::frame_input(output, pipeline, sampling)?;
                (
                    ProcessingRaster::Frame(input),
                    ValidOutput::Extraction {
                        size: output.output,
                        offset,
                    },
                )
            }
        };
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
        Ok(Self {
            output,
            sampling,
            pipeline: pipeline.clone(),
            input: required,
            pitch,
            read,
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
}
