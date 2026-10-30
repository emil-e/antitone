//! Readout representation for ungraded AP1 whose full-domain bound needs folded exposure.

use super::*;
use crate::{
    gpu::{GpuFlatField, GpuInputProgram},
    processing::RollSettings,
    renderer::GpuRenderer,
};

#[cfg(all(test, feature = "editor"))]
mod tests;

pub(crate) struct NeutralReadout {
    pub image: Arc<ImageNode>,
    pub decode_gain: f64,
}

impl NeutralReadout {
    /// Decode individual f32 AP1 values on the GPU. A reduction can instead decode its f64
    /// statistic, retaining means outside f32 without imposing another image boundary.
    pub(crate) fn decoded(&self) -> Arc<ImageNode> {
        if self.decode_gain == 1. {
            return self.image.clone();
        }
        pointwise(
            self.image.clone(),
            Meaning::Neutral,
            Function::new(
                include_str!("neutral_readout.wgsl"),
                "decode_neutral_readout",
            )
            .with_uniform(9, &[self.decode_gain as f32, 0., 0., 0.]),
        )
    }
}

pub(super) fn readout_roll(program: &GpuInputProgram) -> RollSettings {
    let mut roll = RollSettings::default();
    if program.neutral_parameters().is_err() {
        // This is a numerical readout encoding, not the photograph's Roll. Keep it solely
        // dependent on the neutral content so repair values cannot change with grading.
        roll.exposure = -crate::processing::MAX_EXPOSURE;
    }
    roll
}

impl Photograph {
    pub(crate) fn neutral_readout(
        device: &wgpu::Device,
        renderer: &GpuRenderer,
        camera: Arc<ImageNode>,
        field: Option<Arc<GpuFlatField>>,
        program: GpuInputProgram,
    ) -> Result<NeutralReadout, Error> {
        let roll = readout_roll(&program);
        let image =
            Self::unrepaired_readout(device, &renderer.camera, camera, field, program, roll)?;

        Ok(NeutralReadout {
            image,
            decode_gain: (-roll.exposure).exp2(),
        })
    }
}
