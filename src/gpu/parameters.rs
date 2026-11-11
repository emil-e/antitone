//! Validated shader constants for photographic processing.

use crate::{
    Error,
    color::wb_matrix,
    processing::{
        InputSettings, NegativeInput, RollSettings, exposure_multiplier, validate_camera_matrix,
    },
};
use bytemuck::Zeroable;
use glam::{DMat3, DVec3, Vec4};

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ProcessingParameters {
    pub(crate) dmin: Vec4,
    pub(crate) dmax: Vec4,
    pub(crate) gamma_mode: Vec4,
    pub(crate) anchor: Vec4,
    pub(crate) post: [Vec4; 3],
    pub(crate) shape: Vec4,
    pub(crate) tail: [Vec4; 3],
    /// Named input stage for the shader; `STAGE_SOURCE` also means "not graded".
    pub(crate) stage: u32,
    _stage_padding: [u32; 3],
}

pub(crate) const STAGE_SOURCE: u32 = 0;
pub(crate) const STAGE_NEGATIVE: u32 = 1;
pub(crate) const STAGE_DIGITAL: u32 = 2;

// A common ACES source-display convention, not a camera exposure calibration or JPEG fit.
const SOURCE_PREVIEW_EXPOSURE: f64 = 1.0;

pub(crate) fn source_preview_gain(white_balance: Option<[f64; 4]>) -> f64 {
    let headroom = white_balance.and_then(|wb| {
        let gain = crate::processing::libraw_green_scale(wb).ok()?;
        // Bound preview gain for pathological development WB.
        (gain.is_finite() && gain <= crate::processing::MAX_EXPOSURE.exp2()).then_some(gain)
    });
    headroom.unwrap_or(1.) * SOURCE_PREVIEW_EXPOSURE.exp2()
}

impl ProcessingParameters {
    pub const BYTE_LEN: usize = std::mem::size_of::<Self>();

    /// Whether the result is a positive eligible for adjustment-layer grading.
    pub(crate) fn graded(&self) -> bool {
        self.stage != STAGE_SOURCE
    }

    // Working-view and repair entrypoints share this layout; only Roll fields are populated.
    pub(crate) fn photographic(roll: RollSettings) -> Result<Self, Error> {
        let post = CameraMatrix::new(wb_matrix(roll.wb)?)?;
        let gain = exposure_multiplier(roll.exposure)?;
        let anchor = packed_constants(DVec3::ZERO, gain.ln())?;
        Ok(Self {
            post: post.columns,
            anchor,
            stage: STAGE_DIGITAL,
            ..Self::zeroed()
        })
    }

    pub fn source() -> Self {
        Self {
            dmin: Vec4::ZERO,
            dmax: Vec4::ZERO,
            gamma_mode: Vec4::ZERO,
            anchor: Vec4::ZERO,
            post: matrix_columns(DMat3::IDENTITY),
            tail: [Vec4::ZERO; 3],
            ..Self::zeroed()
        }
    }

    /// Display-only source convention; inspection EV is relative to the preview baseline.
    /// Pass WB only for LibRaw samples still on its max-normalized scale.
    /// `DevelopedSource` and SDK samples use `None`.
    pub fn source_preview(white_balance: Option<[f64; 4]>, exposure: f64) -> Result<Self, Error> {
        Self::source_preview_with_transform(white_balance, exposure, DMat3::IDENTITY)
    }

    pub(crate) fn source_preview_with_transform(
        white_balance: Option<[f64; 4]>,
        exposure: f64,
        transform: DMat3,
    ) -> Result<Self, Error> {
        let gain = source_preview_gain(white_balance) * exposure_multiplier(exposure)?;
        Self::source_with_matrix(transform * gain)
    }

    pub fn source_inspection(exposure: f64) -> Result<Self, Error> {
        Self::source_with_gain(exposure_multiplier(exposure)?)
    }

    fn source_with_gain(gain: f64) -> Result<Self, Error> {
        Self::source_with_matrix(DMat3::IDENTITY * gain)
    }

    pub(crate) fn source_with_matrix(matrix: DMat3) -> Result<Self, Error> {
        let post = CameraMatrix::new(matrix)?;
        Ok(Self {
            post: post.columns,
            ..Self::source()
        })
    }

    pub fn positive(
        negative: impl Into<NegativeInput>,
        roll: RollSettings,
        camera_to_ap1: DMat3,
    ) -> Result<Self, Error> {
        let negative = negative.into();
        validate_camera_matrix(camera_to_ap1)?;
        CameraMatrix::new(camera_to_ap1)?;
        let response = negative.parameters()?;
        let gain = exposure_multiplier(roll.exposure)?;
        let matrix = wb_matrix(roll.wb)?;
        let (toe, shoulder) = negative.response.shape()?;
        let reference = packed_constants(negative.calibration.dmin, 0.)?;
        let dmax = packed_constants(negative.calibration.dmax, 0.)?;
        if (0..3).any(|channel| reference[channel] <= dmax[channel]) {
            return Err(Error::Gpu("film anchors coincide after f32 packing".into()));
        }
        let gamma = packed_constants(negative.channel_gammas()?, 0.)?;
        let anchor = packed_constants(response.anchor_logs, gain.ln())?;
        let mut tail = [Vec4::ZERO; 3];
        for (output, &[density, value, slope]) in tail.iter_mut().zip(&response.tail) {
            *output = packed_constants(DVec3::new(density, value * gain, slope * gain), 0.)?;
        }
        let post = CameraMatrix::new(matrix)?;
        let result = Self {
            dmin: reference,
            dmax,
            gamma_mode: gamma,
            shape: Vec4::new(toe as f32, shoulder as f32, 0., 0.),
            tail,
            anchor,
            post: post.columns,
            stage: STAGE_NEGATIVE,
            _stage_padding: [0; 3],
        };
        // GPU arithmetic may flush subnormal references to zero before division/logarithms.
        if !result.dmin.is_finite()
            || result.dmin.truncate().min_element() < f32::MIN_POSITIVE
            || !result.gamma_mode.is_finite()
            || !result.anchor.is_finite()
            || result.post.iter().any(|v| !v.is_finite())
            || !result.shape.is_finite()
        {
            return Err(Error::Gpu(
                "processing parameters cannot be represented in f32".into(),
            ));
        }
        Ok(result)
    }

    /// Digital interpretation is identity; Roll exposure rides in the post-conversion matrix.
    pub fn digital(roll: RollSettings, camera_to_ap1: DMat3) -> Result<Self, Error> {
        validate_camera_matrix(camera_to_ap1)?;
        CameraMatrix::new(camera_to_ap1)?;
        let gain = exposure_multiplier(roll.exposure)?;
        let post = CameraMatrix::new(wb_matrix(roll.wb)? * gain)?;
        Ok(Self {
            post: post.columns,
            stage: STAGE_DIGITAL,
            ..Self::zeroed()
        })
    }

    /// Build packed parameters, interpreting source samples through `stage`.
    pub fn interpret(
        input: impl Into<InputSettings>,
        roll: RollSettings,
        camera_to_ap1: DMat3,
    ) -> Result<Self, Error> {
        let input = input.into();
        match input {
            InputSettings::Digital => Self::digital(roll, camera_to_ap1),
            InputSettings::Negative { .. } => {
                let negative = input.negative_input().ok_or(Error::InvalidCalibration {
                    channel: 0,
                    reason: "measure Dmin and Dmax",
                })?;
                Self::positive(negative, roll, camera_to_ap1)
            }
        }
    }
}

pub(crate) fn input_parameters(
    input: InputSettings,
    matrix: DMat3,
) -> Result<ProcessingParameters, Error> {
    ProcessingParameters::interpret(input, RollSettings::default(), matrix)
}

pub(crate) fn packed_constants(value: DVec3, w: f64) -> Result<Vec4, Error> {
    let packed = value.as_vec3().extend(w as f32);
    if !packed.is_finite() {
        return Err(Error::Gpu(
            "processing constants exceed the f32 range".into(),
        ));
    }
    Ok(packed)
}

pub(crate) fn matrix_columns(matrix: DMat3) -> [Vec4; 3] {
    [matrix.x_axis, matrix.y_axis, matrix.z_axis].map(|v| v.as_vec3().extend(0.0))
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct CameraMatrix {
    pub(crate) columns: [Vec4; 3],
}

impl CameraMatrix {
    pub(crate) fn require_source(matrix: DMat3, continuous: bool) -> Result<(), Error> {
        crate::processing::validate_camera_matrix(matrix)?;
        Self::new(matrix)?;
        if continuous {
            Self::new(matrix * crate::processing::MAX_SOURCE_GAIN)?;
        }
        Ok(())
    }

    pub(crate) fn new(matrix: DMat3) -> Result<Self, Error> {
        let columns = matrix_columns(matrix);
        if columns.iter().any(|v| !v.is_finite()) {
            return Err(Error::Gpu(
                "camera matrix cannot be represented for GPU conversion".into(),
            ));
        }
        Ok(Self { columns })
    }
}
