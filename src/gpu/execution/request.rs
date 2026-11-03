//! Captured processing and sample-storage choices, independent of file encoding.

use crate::{
    Error,
    adjustment_layers::AdjustmentLayerSnapshot,
    geometry::Composition,
    gpu::flat::GpuFlatField,
    pixel_pipeline::PixelPipeline,
    processing::{InputSettings, RollSettings},
    rendering::{OutputReference, OutputTarget},
    scan::Scan,
};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SampleEncoding {
    LinearFloat,
    Rgb16,
}

pub(crate) enum OutputBlock {
    Linear(Vec<[f32; 4]>),
    Rgb16 {
        target: OutputTarget,
        words: Vec<[u32; 2]>,
    },
}

impl OutputBlock {
    pub(super) fn validate(&self) -> Result<(), Error> {
        match self {
            Self::Linear(pixels) => {
                if let Some(pixel) = pixels
                    .iter()
                    .find(|pixel| pixel[3] < 0. || pixel.iter().any(|channel| !channel.is_finite()))
                {
                    return Err(Error::Gpu(format!(
                        "processing reconstruction invalid contributing input/support marker={}",
                        pixel[3],
                    )));
                }
                Ok(())
            }
            Self::Rgb16 { words, .. } => crate::gpu::rgb16::validate(words),
        }
    }
}

/// Source/photographic intent and raster demand. Publication rights stay with the consumer.
pub(crate) struct Request<'a> {
    pub scan: &'a Scan,
    pub field: Option<Arc<GpuFlatField>>,
    pub input: InputSettings,
    pub roll: RollSettings,
    pub grade: AdjustmentLayerSnapshot,
    pub repairs: &'a [Arc<crate::dust_repair::EvaluatedRepair>],
    pub composition: Composition,
    pub stages: PixelPipeline,
    pub output: OutputReference,
    pub luts: crate::looks::LutChain,
    pub encoding: SampleEncoding,
}
