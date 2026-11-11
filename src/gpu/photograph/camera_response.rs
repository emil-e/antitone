//! Pointwise native-camera response, shared by gamma picking and statistical model evaluation.

use super::*;
use crate::gpu::photograph::PhotographGraph;
use crate::{gpu::interpretation::InputTransform, processing::NegativeInput};

#[cfg(test)]
mod tests;

pub(crate) fn normalized(camera: Value, negative: NegativeInput) -> Result<Value, Error> {
    if camera.domain().meaning != Meaning::Camera {
        return Err(Error::InvalidParameter(
            "native response requires normalized camera light",
        ));
    }
    let program = InputTransform::new(negative, glam::DMat3::IDENTITY)?;
    PhotographGraph::native_positive(camera, program)
}
