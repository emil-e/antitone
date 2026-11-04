//! Packed shader constants and numerical admission for photographic processing.

use crate::{
    Error,
    color::wb_matrix,
    processing::{
        InputSettings, NegativeInput, RollSettings, exposure_multiplier, log_abs_expm1,
        validate_camera_matrix,
    },
};
use bytemuck::Zeroable;
use glam::{DMat3, DVec3, Vec4};

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ProcessingParameters {
    pub(crate) dmin_floor: Vec4,
    pub(crate) gamma_mode: Vec4,
    pub(crate) anchor: Vec4,
    pub(crate) post: [Vec4; 3],
    pub(crate) shape: Vec4,
    /// Named input stage for the shader; `STAGE_SOURCE` also means "not graded".
    pub(crate) stage: u32,
    _stage_padding: [u32; 3],
}

pub(crate) const STAGE_SOURCE: u32 = 0;
pub(crate) const STAGE_NEGATIVE: u32 = 1;
pub(crate) const STAGE_DIGITAL: u32 = 2;

// A common ACES source-display convention, not a camera exposure calibration or JPEG fit.
const SOURCE_PREVIEW_EXPOSURE: f64 = 1.0;

pub(crate) fn source_preview_gain(fixed_wb: Option<[f64; 4]>) -> f64 {
    let headroom = fixed_wb.and_then(|wb| {
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
            dmin_floor: Vec4::ZERO,
            gamma_mode: Vec4::ZERO,
            anchor: Vec4::ZERO,
            post: matrix_columns(DMat3::IDENTITY),
            ..Self::zeroed()
        }
    }

    /// Display-only source convention; inspection EV is relative to the preview baseline.
    /// Pass WB only for LibRaw samples still on its max-normalized scale.
    /// `DevelopedSource` and SDK samples use `None`.
    pub fn source_preview(fixed_wb: Option<[f64; 4]>, exposure: f64) -> Result<Self, Error> {
        Self::source_preview_with_transform(fixed_wb, exposure, DMat3::IDENTITY)
    }

    pub(crate) fn source_preview_with_transform(
        fixed_wb: Option<[f64; 4]>,
        exposure: f64,
        transform: DMat3,
    ) -> Result<Self, Error> {
        let gain = source_preview_gain(fixed_wb) * exposure_multiplier(exposure)?;
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
        Self::positive_with_bound(negative, roll, camera_to_ap1).map(|(parameters, _)| parameters)
    }

    pub(crate) fn positive_with_bound(
        negative: impl Into<NegativeInput>,
        roll: RollSettings,
        camera_to_ap1: DMat3,
    ) -> Result<(Self, f64), Error> {
        Self::positive_with_neutral_bound(negative.into(), roll, camera_to_ap1, DVec3::ZERO)
    }

    fn positive_with_neutral_bound(
        negative: NegativeInput,
        roll: RollSettings,
        camera_to_ap1: DMat3,
        additional_neutral: DVec3,
    ) -> Result<(Self, f64), Error> {
        validate_camera_matrix(camera_to_ap1)?;
        CameraMatrix::new(camera_to_ap1)?;
        let response = negative.parameters()?;
        let gain = exposure_multiplier(roll.exposure)?;
        let matrix = wb_matrix(roll.wb)?;
        let (toe, shoulder) = negative.response.shape()?;
        let reference =
            packed_constants(negative.calibration.dmin, negative.calibration.input_floor)?;
        let gamma = packed_constants(negative.channel_gammas()?, 0.)?;
        let anchor = packed_constants(response.anchor_exponents.map(log_abs_expm1), gain.ln())?;
        let mut packed_floor_bound = DVec3::ZERO;
        for c in 0..3 {
            let density = (reference[c] / reference.w).ln();
            let denominator = gamma[c] - shoulder as f32 * density / std::f32::consts::LN_10;
            if !denominator.is_finite() || denominator <= 0. {
                return Err(Error::Gpu(
                    "film response has a pole in its f32 sample domain".into(),
                ));
            }
            // Scalar bounds alone can miss overflow/underflow after packing near a pole.
            // Match the shader's ordinary-f32 floor evaluation, including folded exposure.
            let a = toe as f32 * density / denominator;
            let numerator = if a > 50. {
                a
            } else {
                (a.exp() - 1.).abs().ln()
            };
            let value = ((numerator - anchor[c]) / toe as f32 + anchor.w).exp();
            if !value.is_finite() || value < f32::MIN_POSITIVE {
                return Err(Error::Gpu(
                    "film response exceeds the f32 range at the packed sample floor".into(),
                ));
            }
            packed_floor_bound[c] = f64::from(value);
        }
        let floor_bound = response.floor_bound()?;
        // Above-base continuation is bounded by |expm1(a)| <= 1 for a <= 0.
        let negative_bound = response
            .anchor_exponents
            .map(|a| (-log_abs_expm1(a) / toe).exp());
        let largest = (floor_bound.max(negative_bound) * gain).max(packed_floor_bound);
        let working_bound = (camera_to_ap1.x_axis.abs() * largest.x
            + camera_to_ap1.y_axis.abs() * largest.y
            + camera_to_ap1.z_axis.abs() * largest.z)
            .max(additional_neutral * gain);
        let bound = matrix.x_axis.abs() * working_bound.x
            + matrix.y_axis.abs() * working_bound.y
            + matrix.z_axis.abs() * working_bound.z;
        if !largest.is_finite()
            || largest.max_element() > f32::MAX as f64 / 4.0
            || !working_bound.is_finite()
            || working_bound.max_element() > f32::MAX as f64 / 4.0
            || !bound.is_finite()
            || bound.max_element() > f32::MAX as f64 / 4.0
        {
            return Err(Error::Gpu(
                "settings can exceed the f32 working range in response domain or photographic processing"
                    .into(),
            ));
        }
        let post = CameraMatrix::new(matrix)?;
        let result = Self {
            dmin_floor: reference,
            gamma_mode: gamma,
            shape: Vec4::new(toe as f32, shoulder as f32, 0., 0.),
            anchor,
            post: post.columns,
            stage: STAGE_NEGATIVE,
            _stage_padding: [0; 3],
        };
        if !result.dmin_floor.is_finite()
            || result.dmin_floor.w < f32::MIN_POSITIVE
            || !result.gamma_mode.is_finite()
            || !result.anchor.is_finite()
            || result.post.iter().any(|v| !v.is_finite())
            || !result.shape.is_finite()
        {
            return Err(Error::Gpu(
                "processing parameters cannot be represented in f32".into(),
            ));
        }
        Ok((result, bound.max_element()))
    }

    /// Digital interpretation is identity; Roll exposure rides in the post-conversion matrix.
    pub fn digital(roll: RollSettings, camera_to_ap1: DMat3) -> Result<Self, Error> {
        Self::digital_with_neutral_bound(roll, camera_to_ap1, DVec3::ZERO).map(|(p, _)| p)
    }

    /// Like `digital`, but also returns the f32 working bound for adjustment-layer preflight.
    pub fn digital_with_neutral_bound(
        roll: RollSettings,
        camera_to_ap1: DMat3,
        additional_neutral: DVec3,
    ) -> Result<(Self, f64), Error> {
        validate_camera_matrix(camera_to_ap1)?;
        CameraMatrix::new(camera_to_ap1)?;
        let gain = exposure_multiplier(roll.exposure)?;
        let matrix = wb_matrix(roll.wb)? * gain;
        let post = CameraMatrix::new(matrix)?;
        // With no inversion continuation, the camera-positive bound is the source magnitude.
        let largest = DVec3::splat(crate::processing::MAX_SOURCE_GAIN);
        let working_bound = (camera_to_ap1.x_axis.abs() * largest.x
            + camera_to_ap1.y_axis.abs() * largest.y
            + camera_to_ap1.z_axis.abs() * largest.z)
            .max(additional_neutral);
        let bound = matrix.x_axis.abs() * working_bound.x
            + matrix.y_axis.abs() * working_bound.y
            + matrix.z_axis.abs() * working_bound.z;
        if !working_bound.is_finite()
            || working_bound.max_element() > f32::MAX as f64 / 4.0
            || !bound.is_finite()
            || bound.max_element() > f32::MAX as f64 / 4.0
        {
            return Err(Error::Gpu(
                "settings can exceed the f32 working range in digital processing".into(),
            ));
        }
        Ok((
            Self {
                post: post.columns,
                stage: STAGE_DIGITAL,
                ..Self::zeroed()
            },
            bound.max_element(),
        ))
    }

    /// Like `interpret`, but returns the f32 working bound (adjustment-layer preflight).
    pub fn interpret_with_neutral_bound(
        input: impl Into<InputSettings>,
        roll: RollSettings,
        camera_to_ap1: DMat3,
        additional_neutral: DVec3,
    ) -> Result<(Self, f64), Error> {
        let input = input.into();
        match input {
            InputSettings::Digital => {
                Self::digital_with_neutral_bound(roll, camera_to_ap1, additional_neutral)
            }
            InputSettings::Negative { .. } => {
                let negative = input.negative_input().ok_or(Error::InvalidCalibration {
                    channel: 0,
                    reason: "measure Dmin and Dmax",
                })?;
                Self::positive_with_neutral_bound(negative, roll, camera_to_ap1, additional_neutral)
            }
        }
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
        let bound = matrix.x_axis.abs() + matrix.y_axis.abs() + matrix.z_axis.abs();
        if !bound.is_finite()
            || bound.max_element() > f32::MAX as f64 / 16.
            || columns.iter().any(|v| !v.is_finite())
        {
            return Err(Error::Gpu(
                "camera matrix cannot be represented for GPU conversion".into(),
            ));
        }
        Ok(Self { columns })
    }
}
