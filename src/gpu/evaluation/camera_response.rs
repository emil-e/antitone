//! Pointwise native-camera response, shared by gamma picking and statistical model evaluation.

use super::*;
use crate::{gpu::GpuInputProgram, processing::NegativeInput};

#[cfg(test)]
mod tests;

pub(crate) struct CameraReadout {
    pub image: Arc<ImageNode>,
    pub decode_gain: f64,
}

impl CameraReadout {
    pub(crate) fn normalized(
        camera: Arc<ImageNode>,
        negative: NegativeInput,
    ) -> Result<Self, Error> {
        if camera.domain.meaning != Meaning::Camera {
            return Err(Error::InvalidParameter(
                "native response requires normalized camera light",
            ));
        }

        let program = GpuInputProgram::new(negative, glam::DMat3::IDENTITY)?;
        let roll = super::neutral_readout::readout_roll(&program);
        let image = Photograph::native_positive(camera, program, roll)?;

        Ok(Self {
            image,
            decode_gain: (-roll.exposure).exp2(),
        })
    }
}
