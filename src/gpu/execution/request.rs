//! Captured processing and sample-storage choices, independent of file encoding.
use crate::{
    Error,
    adjustment_layers::AdjustmentLayerSnapshot,
    geometry::Composition,
    gpu::GpuFlatField,
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

/// Maximum original-cell work in one submission. Output units are bounded separately.
#[derive(Clone, Copy)]
pub(crate) struct WorkBudget {
    pub source_pixels: u64,
}

impl WorkBudget {
    pub(super) fn require_source(&self, size: [u32; 2]) -> Result<(), Error> {
        let count = u64::from(size[0]) * u64::from(size[1]);
        if count > self.source_pixels || count > u64::from(u32::MAX) {
            return Err(Error::InvalidParameter(
                "processing unit exceeds source work budget",
            ));
        }
        Ok(())
    }

    pub fn for_device(device: &wgpu::Device) -> Self {
        let limits = device.limits();
        Self {
            source_pixels: limits
                .max_buffer_size
                .min(limits.max_storage_buffer_binding_size)
                / 16,
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
    pub look: Option<Arc<crate::looks::LookAsset>>,
    pub encoding: SampleEncoding,
    pub budget: WorkBudget,
}
