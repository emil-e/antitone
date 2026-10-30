use bytemuck::Zeroable;
use std::sync::Arc;

#[cfg(any(feature = "render", feature = "project"))]
pub(crate) mod aces;
#[cfg(feature = "editor")]
pub(crate) mod analysis;
pub mod evaluation;
#[cfg(all(feature = "render", any(feature = "raw", feature = "ui")))]
pub(crate) mod execution;
mod flat;
pub mod look;
#[cfg(all(
    feature = "raw",
    feature = "render",
    feature = "project",
    any(feature = "editor", feature = "export")
))]
pub(crate) mod repair_source;
#[cfg(all(feature = "raw", feature = "render"))]
pub(crate) mod rgb16;
#[cfg(all(test, feature = "render"))]
pub(crate) mod test_support;
pub use flat::{GpuFlatField, SourceGeometry};
mod grading;
pub use grading::Grading;
mod input_program;
pub use input_program::{GpuInputProgram, InputSignature};
mod photograph;

pub(crate) mod grain;
pub(crate) mod image_mtf;
#[cfg(any(
    feature = "editor",
    all(feature = "ui", feature = "render"),
    all(feature = "raw", feature = "render")
))]
pub(crate) mod pixel_pipeline;
pub(crate) mod raster;

mod input;
pub use input::{ProcessingInput, ProcessingSignature};
mod submission;
#[cfg(all(test, target_os = "macos", feature = "raw", feature = "export"))]
pub(crate) mod timestamp_ledger;
pub use submission::SubmissionGate;
#[cfg(feature = "editor")]
pub(crate) mod view;

use glam::{DMat3, DVec3, Vec4};
use wgpu::util::DeviceExt;

use crate::Error;
use crate::color::wb_matrix;
use crate::processing::{
    InputSettings, NegativeInput, RollSettings, exposure_multiplier, log_abs_expm1,
    validate_camera_matrix,
};

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ProcessingParameters {
    dmin_floor: Vec4,
    gamma_mode: Vec4,
    anchor: Vec4,
    post: [Vec4; 3],
    shape: Vec4,
    /// Named input stage for the shader; `STAGE_SOURCE` also means "not graded".
    stage: u32,
    _stage_padding: [u32; 3],
}

pub(crate) const STAGE_SOURCE: u32 = 0;
pub(crate) const STAGE_NEGATIVE: u32 = 1;
pub(crate) const STAGE_DIGITAL: u32 = 2;

// A common ACES source-display convention, not a camera exposure calibration or JPEG fit.
const SOURCE_PREVIEW_EXPOSURE: f64 = 1.0;

fn source_preview_gain(fixed_wb: Option<[f64; 4]>) -> f64 {
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
    pub(super) fn photographic(roll: RollSettings) -> Result<Self, Error> {
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

    fn source_with_matrix(matrix: DMat3) -> Result<Self, Error> {
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

    fn positive_with_bound(
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

fn input_parameters(input: InputSettings, matrix: DMat3) -> Result<ProcessingParameters, Error> {
    ProcessingParameters::interpret(input, RollSettings::default(), matrix)
}

fn packed_constants(value: DVec3, w: f64) -> Result<Vec4, Error> {
    let packed = value.as_vec3().extend(w as f32);
    if !packed.is_finite() {
        return Err(Error::Gpu(
            "processing constants exceed the f32 range".into(),
        ));
    }
    Ok(packed)
}

fn matrix_columns(matrix: DMat3) -> [Vec4; 3] {
    [matrix.x_axis, matrix.y_axis, matrix.z_axis].map(|v| v.as_vec3().extend(0.0))
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraMatrix {
    columns: [Vec4; 3],
}

impl CameraMatrix {
    fn new(matrix: DMat3) -> Result<Self, Error> {
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

pub(crate) mod repairs;

pub struct ProcessingSource {
    input: wgpu::Buffer,
    matrix: wgpu::Buffer,
    pixels: u32,
    camera_to_ap1: DMat3,
    geometry: Option<SourceGeometry>,
    flat_field: Option<Arc<GpuFlatField>>,
    continuous: bool,
}

impl ProcessingSource {
    #[cfg(feature = "editor")]
    pub(crate) fn with_matrix(
        self: &Arc<Self>,
        device: &wgpu::Device,
        matrix: DMat3,
    ) -> Result<Arc<Self>, Error> {
        if self.camera_to_ap1 == matrix {
            return Ok(self.clone());
        }
        validate_camera_matrix(matrix)?;
        let mut packed = CameraMatrix::new(matrix)?;
        if self.continuous {
            CameraMatrix::new(matrix * crate::processing::MAX_SOURCE_GAIN)?;
        }
        packed.columns[0].w = if self.continuous { 1. } else { 0. };
        Ok(Arc::new(Self {
            input: self.input.clone(),
            matrix: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("normalized camera-to-AP1 source transform"),
                contents: bytemuck::bytes_of(&packed),
                usage: wgpu::BufferUsages::UNIFORM,
            }),
            pixels: self.pixels,
            camera_to_ap1: matrix,
            geometry: self.geometry,
            flat_field: self.flat_field.clone(),
            continuous: self.continuous,
        }))
    }

    pub fn new(device: &wgpu::Device, pixels: &[[u16; 4]], matrix: DMat3) -> Result<Self, Error> {
        Self::from_bytes(
            device,
            bytemuck::cast_slice(pixels),
            pixels.len(),
            matrix,
            false,
        )
    }

    /// Continuous camera RGB in native code units, not rounded or packed into RGB16.
    pub fn new_continuous(
        device: &wgpu::Device,
        pixels: &[[f32; 4]],
        matrix: DMat3,
    ) -> Result<Self, Error> {
        Self::validate_continuous(pixels)?;
        Self::from_bytes(
            device,
            bytemuck::cast_slice(pixels),
            pixels.len(),
            matrix,
            true,
        )
    }

    fn validate_continuous(pixels: &[[f32; 4]]) -> Result<(), Error> {
        if pixels.iter().flatten().any(|v| {
            !v.is_finite() || !(0. ..=crate::processing::MAX_CAMERA_SOURCE_CODE).contains(v)
        }) {
            return Err(Error::Gpu(
                "continuous camera samples exceed the supported code domain".into(),
            ));
        }
        Ok(())
    }

    fn from_bytes(
        device: &wgpu::Device,
        bytes: &[u8],
        length: usize,
        matrix: DMat3,
        continuous: bool,
    ) -> Result<Self, Error> {
        let count =
            u32::try_from(length).map_err(|_| Error::Gpu("source exceeds GPU limits".into()))?;
        if count == 0
            || bytes.len() as u64 > device.limits().max_storage_buffer_binding_size
            || u64::from(count) * 16 > device.limits().max_storage_buffer_binding_size
        {
            return Err(Error::Gpu("source exceeds GPU limits".into()));
        }
        validate_camera_matrix(matrix)?;
        let camera_to_ap1 = matrix;
        let mut matrix = CameraMatrix::new(matrix)?;
        if continuous {
            CameraMatrix::new(camera_to_ap1 * crate::processing::MAX_SOURCE_GAIN)?;
        }
        // xyz remain colour coefficients; w tags the storage format read by source_codes.
        matrix.columns[0].w = if continuous { 1. } else { 0. };
        Ok(Self {
            input: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("camera source samples"),
                contents: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            }),
            matrix: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("camera to AP1 post-inversion constants"),
                contents: bytemuck::bytes_of(&matrix),
                usage: wgpu::BufferUsages::UNIFORM,
            }),
            pixels: count,
            camera_to_ap1,
            geometry: None,
            flat_field: None,
            continuous,
        })
    }

    pub fn set_geometry(&mut self, geometry: SourceGeometry) -> Result<(), Error> {
        if geometry.pixels() != self.pixels {
            return Err(Error::Gpu(
                "source geometry differs from RGB16 storage".into(),
            ));
        }
        self.geometry = Some(geometry);
        Ok(())
    }

    pub fn set_flat_field(
        &mut self,
        field: Option<Arc<GpuFlatField>>,
        geometry: SourceGeometry,
    ) -> Result<(), Error> {
        if let Some(field) = &field {
            field.validate_matrix(
                self.camera_to_ap1,
                if self.continuous {
                    crate::processing::MAX_SOURCE_GAIN
                } else {
                    1.
                },
            )?;
        }
        self.set_geometry(geometry)?;
        self.flat_field = field;
        Ok(())
    }

    pub fn write(&self, queue: &wgpu::Queue, pixels: &[[u16; 4]]) -> Result<(), Error> {
        if self.continuous || pixels.len() != self.pixels as usize {
            return Err(Error::Gpu(
                "invalid replacement source tile or format".into(),
            ));
        }
        queue.write_buffer(&self.input, 0, bytemuck::cast_slice(pixels));
        Ok(())
    }

    pub fn write_continuous(&self, queue: &wgpu::Queue, pixels: &[[f32; 4]]) -> Result<(), Error> {
        if !self.continuous || pixels.len() != self.pixels as usize {
            return Err(Error::Gpu(
                "invalid replacement continuous source tile".into(),
            ));
        }
        Self::validate_continuous(pixels)?;
        queue.write_buffer(&self.input, 0, bytemuck::cast_slice(pixels));
        Ok(())
    }

    pub fn byte_len(&self) -> u64 {
        self.input.size()
    }

    pub(crate) fn pixel_count(&self) -> u32 {
        self.pixels
    }

    #[cfg(all(feature = "render", any(feature = "ui", feature = "raw")))]
    pub(crate) fn geometry(&self) -> Option<SourceGeometry> {
        self.geometry
    }
}

pub use photograph::CameraResources;

/// Retained GPU resource bytes where the platform can report them; not total process memory.
#[cfg(any(test, feature = "ui"))]
pub(crate) use crate::sys::allocated_resource_bytes;

/// The graphics API this platform presents and computes with, from the one `sys` selection.
pub fn backends() -> wgpu::Backends {
    crate::sys::backends()
}

/// An instance with this platform's graphics API and shader-compiler policy.
pub fn instance() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: crate::sys::backends(),
        backend_options: crate::sys::backend_options(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    })
}

#[cfg(test)]
pub(crate) use evaluation::camera_fixture;
#[cfg(test)]
mod flat_tests;
#[cfg(test)]
mod input_grading_tests;
#[cfg(test)]
mod tests;
