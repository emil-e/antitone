use super::{ProcessingParameters, ProcessingSource};
use crate::{Error, processing::InputSettings};
use glam::DMat3;

/// Input-program content identity; captured source identity is supplied by the source owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputSignature {
    parameters: [u8; ProcessingParameters::BYTE_LEN],
    matrix: [u64; 9],
}

#[cfg(test)]
impl From<[u8; ProcessingParameters::BYTE_LEN]> for InputSignature {
    fn from(parameters: [u8; ProcessingParameters::BYTE_LEN]) -> Self {
        Self {
            parameters,
            matrix: [0; 9],
        }
    }
}

#[cfg(test)]
impl Default for InputSignature {
    fn default() -> Self {
        [0; ProcessingParameters::BYTE_LEN].into()
    }
}

/// Source execution captured separately from photographic controls.
#[derive(Clone, Debug, PartialEq)]
pub struct GpuInputProgram {
    pub(super) input: InputSettings,
    pub(super) matrix: DMat3,
}

impl GpuInputProgram {
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

    #[cfg(feature = "raw")]
    pub(crate) fn for_scan(scan: &crate::scan::Scan, input: InputSettings) -> Result<Self, Error> {
        let input = scan.refined_input(&input)?;
        Self::new(input, scan.positive_matrix(&input)?)
    }

    pub fn signature(&self) -> Result<InputSignature, Error> {
        let parameters = self.neutral_parameters()?;
        Ok(InputSignature {
            parameters: bytemuck::bytes_of(&parameters).try_into().unwrap(),
            matrix: self.matrix.to_cols_array().map(f64::to_bits),
        })
    }

    pub fn source_continuous(
        &self,
        device: &wgpu::Device,
        pixels: &[[f32; 4]],
    ) -> Result<ProcessingSource, Error> {
        ProcessingSource::new_continuous(device, pixels, self.matrix)
    }

    pub(super) fn neutral_parameters(&self) -> Result<ProcessingParameters, Error> {
        super::input_parameters(self.input, self.matrix)
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
