//! Captured photographic input transforms and source-inspection interpretation.

use crate::gpu::{parameters::ProcessingParameters, source::ProcessingSource};
use crate::{Error, processing::InputSettings};
use glam::DMat3;

/// Source execution captured separately from photographic controls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InputTransform {
    pub(super) input: InputSettings,
    pub(super) matrix: DMat3,
}

#[cfg(feature = "editor")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum SourceInterpretation {
    Photograph(InputTransform),
    Inspection(DMat3),
}

#[cfg(feature = "editor")]
impl SourceInterpretation {
    pub(crate) fn matrix(self) -> DMat3 {
        match self {
            Self::Photograph(program) => program.matrix,
            Self::Inspection(matrix) => matrix,
        }
    }

    #[cfg(test)]
    pub(crate) fn transform(&self) -> Option<&InputTransform> {
        match self {
            Self::Photograph(program) => Some(program),
            Self::Inspection(_) => None,
        }
    }
}

impl InputTransform {
    pub fn new(input: impl Into<InputSettings>, matrix: DMat3) -> Result<Self, Error> {
        let input = input.into();
        crate::processing::validate_camera_matrix(matrix)?;
        if !matches!(input, InputSettings::Digital) {
            input
                .negative_input()
                .ok_or(Error::InvalidCalibration {
                    channel: 0,
                    reason: "measure Dmin and Dmax",
                })?
                .parameters()?;
        }
        Ok(Self { input, matrix })
    }

    #[cfg(all(test, feature = "editor"))]
    pub(crate) fn matrix(&self) -> DMat3 {
        self.matrix
    }

    #[cfg(feature = "raw")]
    pub(crate) fn for_scan(scan: &crate::scan::Scan, input: InputSettings) -> Result<Self, Error> {
        let input = scan.refined_input(&input)?;
        Self::new(input, scan.positive_matrix(&input)?)
    }

    pub fn source_continuous(
        &self,
        device: &wgpu::Device,
        pixels: &[[f32; 4]],
    ) -> Result<ProcessingSource, Error> {
        ProcessingSource::new_continuous(device, pixels, self.matrix)
    }

    pub(super) fn neutral_parameters(&self) -> Result<ProcessingParameters, Error> {
        crate::gpu::parameters::input_parameters(self.input, self.matrix)
    }

    #[cfg(feature = "raw")]
    pub(super) fn matches_native(&self, scan: &crate::scan::Scan) -> Result<bool, Error> {
        Ok(scan.positive_matrix(&self.input)? == self.matrix)
    }

    #[cfg(feature = "raw")]
    pub(super) fn matches_proxy(&self, proxy: &crate::scan::CameraProxy) -> Result<bool, Error> {
        Ok(proxy.positive_matrix(&self.input)? == self.matrix)
    }

    pub(super) fn require_source(&self, source: &ProcessingSource) -> Result<(), Error> {
        if source.camera_to_ap1 != self.matrix {
            return Err(Error::Gpu(
                "input program and camera source matrices differ".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_content_is_not_the_packed_shader_encoding() {
        let input = crate::processing::InputSettings::Negative {
            dmin: Some(glam::DVec3::splat(0.8)),
            dmax: Some(glam::DVec3::splat(0.02)),
            input_floor: crate::processing::CAMERA_INPUT_FLOOR,
            response: crate::processing::Response::film(),
            gamma: 0.75,
            balance: Default::default(),
        };
        let a = InputTransform::new(input, DMat3::IDENTITY).unwrap();
        let mut changed = input;
        if let InputSettings::Negative { gamma, .. } = &mut changed {
            *gamma = gamma.next_up();
        }
        let b = InputTransform::new(changed, DMat3::IDENTITY).unwrap();
        let packed_a = a.neutral_parameters().unwrap();
        let packed_b = b.neutral_parameters().unwrap();
        assert_eq!(
            bytemuck::bytes_of(&packed_a),
            bytemuck::bytes_of(&packed_b),
            "control must collide after packing"
        );
        assert_ne!(
            a, b,
            "captured input content must distinguish the original parameters"
        );
        assert_eq!(a, InputTransform::new(input, DMat3::IDENTITY).unwrap());
    }
}
