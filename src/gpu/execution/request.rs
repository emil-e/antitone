//! Captured processing and sample-storage choices, independent of file encoding.

use crate::{Error, geometry::Composition, gpu::evaluation::Value, rendering::OutputTarget};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SampleEncoding {
    LinearFloat,
    Rgb16(OutputTarget),
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

/// A complete calculation and its output footprint. Publication rights stay with the consumer.
pub(crate) struct Request {
    pub definition: Value,
    pub composition: Composition,
    pub encoding: SampleEncoding,
}
