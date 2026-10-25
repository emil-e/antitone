use bytemuck::Zeroable;
use std::{num::NonZeroU64, sync::Arc};

#[path = "gpu_flat_field.rs"]
mod flat;
#[path = "gpu_look.rs"]
pub mod look;
pub use flat::{GpuFlatField, SourceGeometry};
#[path = "gpu_local_adjustments.rs"]
mod local;
pub use local::GpuPhotoGrade;
#[path = "gpu_processing_input.rs"]
mod input;
pub use input::{ProcessingInput, ProcessingSignature};
#[path = "gpu_submission.rs"]
mod submission;
#[cfg(all(test, target_os = "macos", feature = "raw", feature = "export"))]
#[path = "gpu_timestamp_ledger.rs"]
pub(crate) mod timestamp_ledger;
pub use submission::SubmissionGate;
#[cfg(feature = "editor")]
#[path = "gpu_dust_screening.rs"]
pub(crate) mod dust_screening;
#[cfg(feature = "editor")]
#[path = "gpu_working_view.rs"]
pub(crate) mod view;

use glam::{DMat3, DVec3, Vec4};
use wgpu::util::DeviceExt;

use crate::Error;
use crate::color::{AP1_TO_XYZ, wb_matrix};
use crate::processing::{
    InputSettings, Inversion, NegativeInput, PhotoSettings, RollSettings, exposure_multiplier,
    log_abs_expm1, validate_camera_matrix,
};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ProcessingParameters {
    dmin_floor: Vec4,
    gamma_mode: Vec4,
    anchor: Vec4,
    post: [Vec4; 3],
    shape: Vec4,
    creative: [Vec4; 3],
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
        // Match the bridge's f32 gains and pinned LibRaw highlight=1 normalization.
        // Sensor black/white scaling is already in RGB16 and cancels from this ratio.
        let wb = wb.map(|v| v as f32);
        if wb.iter().any(|v| !v.is_finite() || *v <= 0.) {
            return None;
        }
        let maximum = wb.into_iter().fold(0_f32, f32::max);
        if f64::from(maximum) <= 0.00001 {
            // LibRaw does not normalize gains in this degenerate scaling branch.
            return None;
        }
        let gain = f64::from(maximum) / f64::from(wb[1]);
        // Bound preview gain for pathological development WB.
        (gain.is_finite() && gain <= crate::processing::MAX_EXPOSURE.exp2()).then_some(gain)
    });
    headroom.unwrap_or(1.) * SOURCE_PREVIEW_EXPOSURE.exp2()
}

impl ProcessingParameters {
    pub const BYTE_LEN: usize = std::mem::size_of::<Self>();

    /// A graded positive render; source inspection is not graded.
    pub(crate) fn graded(&self) -> bool {
        self.stage != STAGE_SOURCE
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
    /// `fixed_wb` must describe the development, not each photo's as-shot WB.
    pub fn source_preview(fixed_wb: Option<[f64; 4]>, exposure: f64) -> Result<Self, Error> {
        let gain = source_preview_gain(fixed_wb) * exposure_multiplier(exposure)?;
        Self::source_with_gain(gain)
    }

    pub fn source_inspection(exposure: f64) -> Result<Self, Error> {
        Self::source_with_gain(exposure_multiplier(exposure)?)
    }

    fn source_with_gain(gain: f64) -> Result<Self, Error> {
        let matrix = DMat3::IDENTITY * gain;
        let post = CameraMatrix::new(matrix)?;
        Ok(Self {
            post: post.columns,
            ..Self::source()
        })
    }

    pub fn positive(
        negative: impl Into<NegativeInput>,
        roll: RollSettings,
        photo: PhotoSettings,
        camera_to_ap1: DMat3,
    ) -> Result<Self, Error> {
        Self::positive_with_bound(negative, roll, photo, camera_to_ap1)
            .map(|(parameters, _)| parameters)
    }

    fn positive_with_bound(
        negative: impl Into<NegativeInput>,
        roll: RollSettings,
        photo: PhotoSettings,
        camera_to_ap1: DMat3,
    ) -> Result<(Self, f64), Error> {
        Self::positive_with_neutral_bound(negative.into(), roll, photo, camera_to_ap1, DVec3::ZERO)
    }

    fn positive_with_neutral_bound(
        negative: NegativeInput,
        roll: RollSettings,
        photo: PhotoSettings,
        camera_to_ap1: DMat3,
        additional_neutral: DVec3,
    ) -> Result<(Self, f64), Error> {
        validate_camera_matrix(camera_to_ap1)?;
        CameraMatrix::new(camera_to_ap1)?;
        let inversion = Inversion::new(negative)?;
        photo.validate()?;
        let grade = crate::photographic::Grade::new(photo)?;
        let gain = exposure_multiplier(roll.exposure)? * exposure_multiplier(photo.exposure)?;
        let matrix = wb_matrix(photo.wb)? * wb_matrix(roll.wb)?;
        let (toe, shoulder) = negative.response.shape()?;
        let reference =
            packed_constants(negative.calibration.dmin, negative.calibration.input_floor)?;
        let gamma = packed_constants(negative.channel_gammas()?, 0.)?;
        let anchor = packed_constants(inversion.anchor_exponents.map(log_abs_expm1), gain.ln())?;
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
        let floor_bound = inversion
            .apply(DVec3::splat(negative.calibration.input_floor))?
            .rgb
            .abs();
        // Above-base continuation is bounded by |expm1(a)| <= 1 for a <= 0.
        let negative_bound = inversion
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
        let creative_bound = grade.intermediate_bound(bound.max_element());
        if !largest.is_finite()
            || largest.max_element() > f32::MAX as f64 / 4.0
            || !working_bound.is_finite()
            || working_bound.max_element() > f32::MAX as f64 / 4.0
            || !bound.is_finite()
            || bound.max_element() > f32::MAX as f64 / 4.0
            || !creative_bound.is_finite()
            || creative_bound > f32::MAX as f64 / 4.0
        {
            return Err(Error::Gpu(
                "settings can exceed the f32 working range in response domain or photographic processing"
                    .into(),
            ));
        }
        let post = CameraMatrix::new(matrix)?;
        let weights = AP1_TO_XYZ.transpose().y_axis;
        let creative = [
            packed_constants(DVec3::new(photo.contrast, grade.qp, grade.q0), grade.black)?,
            packed_constants(
                DVec3::new(
                    photo.contrast_colour_mix,
                    photo.saturation,
                    photo.saturation_density,
                ),
                0.,
            )?,
            packed_constants(weights, 0.)?,
        ];
        let (_, output) = grade.weighted_bounds(bound.max_element(), 1.);
        let result = Self {
            dmin_floor: reference,
            gamma_mode: gamma,
            shape: Vec4::new(toe as f32, shoulder as f32, 0., 0.),
            anchor,
            post: post.columns,
            creative,
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
        // Scratch maxima must not become the next stage's input. Both bounds
        // cover the actual full-strength output; keep the tighter envelope.
        Ok((result, output.min(creative_bound)))
    }

    /// A digital source is already a positive: interpretation is identity, so there is no
    /// response `exp` to fold the exposure gain into. It rides in `post` instead (matching
    /// the CPU `working_to_graded`), and only camera→AP1 · post · grade run.
    pub fn digital(
        roll: RollSettings,
        photo: PhotoSettings,
        camera_to_ap1: DMat3,
    ) -> Result<Self, Error> {
        Self::digital_with_neutral_bound(roll, photo, camera_to_ap1, DVec3::ZERO).map(|(p, _)| p)
    }

    /// Like `digital`, but also returns the f32 working bound for local-grade preflight.
    pub fn digital_with_neutral_bound(
        roll: RollSettings,
        photo: PhotoSettings,
        camera_to_ap1: DMat3,
        additional_neutral: DVec3,
    ) -> Result<(Self, f64), Error> {
        validate_camera_matrix(camera_to_ap1)?;
        CameraMatrix::new(camera_to_ap1)?;
        photo.validate()?;
        let grade = crate::photographic::Grade::new(photo)?;
        let gain = exposure_multiplier(roll.exposure)? * exposure_multiplier(photo.exposure)?;
        let matrix = wb_matrix(photo.wb)? * wb_matrix(roll.wb)? * gain;
        let post = CameraMatrix::new(matrix)?;
        // f32 preflight: source (already positive) → camera→AP1 → post → grade. With no
        // inversion continuation, the native-positive bound is simply the source magnitude.
        let largest = DVec3::splat(crate::processing::MAX_SOURCE_GAIN);
        let working_bound = (camera_to_ap1.x_axis.abs() * largest.x
            + camera_to_ap1.y_axis.abs() * largest.y
            + camera_to_ap1.z_axis.abs() * largest.z)
            .max(additional_neutral * gain);
        let bound = matrix.x_axis.abs() * working_bound.x
            + matrix.y_axis.abs() * working_bound.y
            + matrix.z_axis.abs() * working_bound.z;
        let creative_bound = grade.intermediate_bound(bound.max_element());
        if !working_bound.is_finite()
            || working_bound.max_element() > f32::MAX as f64 / 4.0
            || !bound.is_finite()
            || bound.max_element() > f32::MAX as f64 / 4.0
            || !creative_bound.is_finite()
            || creative_bound > f32::MAX as f64 / 4.0
        {
            return Err(Error::Gpu(
                "settings can exceed the f32 working range in digital processing".into(),
            ));
        }
        let weights = AP1_TO_XYZ.transpose().y_axis;
        let creative = [
            packed_constants(DVec3::new(photo.contrast, grade.qp, grade.q0), grade.black)?,
            packed_constants(
                DVec3::new(
                    photo.contrast_colour_mix,
                    photo.saturation,
                    photo.saturation_density,
                ),
                0.,
            )?,
            packed_constants(weights, 0.)?,
        ];
        let (_, output) = grade.weighted_bounds(bound.max_element(), 1.);
        Ok((
            Self {
                post: post.columns,
                creative,
                stage: STAGE_DIGITAL,
                ..Self::zeroed()
            },
            output.min(creative_bound),
        ))
    }

    /// Like `interpret`, but returns the f32 working bound (local-grade preflight).
    pub fn interpret_with_neutral_bound(
        input: impl Into<InputSettings>,
        roll: RollSettings,
        photo: PhotoSettings,
        camera_to_ap1: DMat3,
        additional_neutral: DVec3,
    ) -> Result<(Self, f64), Error> {
        let input = input.into();
        match input {
            InputSettings::Digital => {
                Self::digital_with_neutral_bound(roll, photo, camera_to_ap1, additional_neutral)
            }
            InputSettings::Negative { .. } => {
                let negative = input.negative_input().ok_or(Error::InvalidCalibration {
                    channel: 0,
                    reason: "measure Dmin and Dmax",
                })?;
                Self::positive_with_neutral_bound(
                    negative,
                    roll,
                    photo,
                    camera_to_ap1,
                    additional_neutral,
                )
            }
        }
    }

    /// Build packed parameters, interpreting source samples through `stage`.
    pub fn interpret(
        input: impl Into<InputSettings>,
        roll: RollSettings,
        photo: PhotoSettings,
        camera_to_ap1: DMat3,
    ) -> Result<Self, Error> {
        let input = input.into();
        match input {
            InputSettings::Digital => Self::digital(roll, photo, camera_to_ap1),
            InputSettings::Negative { .. } => {
                let negative = input.negative_input().ok_or(Error::InvalidCalibration {
                    channel: 0,
                    reason: "measure Dmin and Dmax",
                })?;
                Self::positive(negative, roll, photo, camera_to_ap1)
            }
        }
    }
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

#[cfg(any(feature = "editor", all(feature = "raw", feature = "export")))]
#[path = "gpu_repairs.rs"]
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

    #[cfg(feature = "editor")]
    pub(crate) fn geometry(&self) -> Option<SourceGeometry> {
        self.geometry
    }
}

pub struct ProcessingKernel {
    #[cfg(feature = "editor")]
    module: wgpu::ShaderModule,
    pipeline: wgpu::ComputePipeline,
    repair: wgpu::ComputePipeline,
    output_binding: u32,
    output_stride: u64,
    workgroup_size: u32,
    identity: wgpu::Buffer,
    empty_locals: wgpu::Buffer,
    empty_local_context: wgpu::Buffer,
}

impl ProcessingKernel {
    pub async fn new(device: &wgpu::Device) -> Result<Self, Error> {
        Self::build_pipeline(device, Self::module_shader()).await
    }

    /// The exact module text the pipelines compile, exposed for device-free validation.
    pub(crate) fn module_shader() -> String {
        [
            Self::kernel_shader(),
            include_str!("shaders/footprint.wgsl").to_owned(),
            include_str!("shaders/repairs.wgsl").to_owned(),
        ]
        .join("\n")
    }

    /// One implementation serves the viewport, export and analysis: the near-base ratio response
    /// and the f32 colour operators.
    pub(crate) fn kernel_shader() -> String {
        let common = [
            include_str!("shaders/camera_conversion.wgsl"),
            include_str!("shaders/flat_field.wgsl"),
            include_str!("shaders/film_response.wgsl"),
            include_str!("shaders/film_inversion_export.wgsl"),
            include_str!("shaders/photographic.wgsl"),
            include_str!("shaders/local_adjustments.wgsl"),
            include_str!("shaders/photographic_colour.wgsl"),
            include_str!("shaders/photographic_export.wgsl"),
        ]
        .join("\n");
        [common.as_str(), include_str!("shaders/processing.wgsl")].join("\n")
    }

    async fn build_pipeline(device: &wgpu::Device, shader: String) -> Result<Self, Error> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("camera originals to AP1 photographic processing"),
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Antitone processing"),
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let repair = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("source-coordinate repair grading"),
            layout: None,
            module: &module,
            entry_point: Some("repair_original"),
            compilation_options: Default::default(),
            cache: None,
        });
        if let Some(error) = scope.pop().await {
            return Err(Error::Gpu(error.to_string()));
        }
        Ok(Self {
            #[cfg(feature = "editor")]
            module,
            pipeline,
            repair,
            output_binding: 1,
            output_stride: 16,
            workgroup_size: 64,
            identity: flat::identity_nodes(device),
            empty_locals: local::empty_buffer(device),
            empty_local_context: local::context_buffer(device, 0, [0; 4])?,
        })
    }

    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &ProcessingSource,
        output: &wgpu::Buffer,
        parameters: ProcessingParameters,
    ) {
        self.encode_resources(
            device,
            encoder,
            source,
            output,
            parameters,
            (&self.empty_locals, &self.empty_local_context),
            #[cfg(test)]
            (None, None),
        );
    }

    pub fn encode_grade(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &ProcessingSource,
        output: &wgpu::Buffer,
        grade: &GpuPhotoGrade,
    ) -> Result<(), Error> {
        grade.require_source(source)?;
        self.encode_resources(
            device,
            encoder,
            source,
            output,
            grade.parameters,
            (&grade.buffer, &grade.context),
            #[cfg(test)]
            (None, None),
        );
        Ok(())
    }

    #[cfg(all(test, target_os = "macos", feature = "raw", feature = "export"))]
    pub(crate) fn encode_grade_with_query(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &ProcessingSource,
        output: &wgpu::Buffer,
        grade: &GpuPhotoGrade,
        queries: (&wgpu::QuerySet, u32),
    ) -> Result<(), Error> {
        grade.require_source(source)?;
        self.encode_resources(
            device,
            encoder,
            source,
            output,
            grade.parameters,
            (&grade.buffer, &grade.context),
            (Some(queries), None),
        );
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_resources(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &ProcessingSource,
        output: &wgpu::Buffer,
        parameters: ProcessingParameters,
        (locals, context): (&wgpu::Buffer, &wgpu::Buffer),
        #[cfg(test)] diagnostics: (Option<(&wgpu::QuerySet, u32)>, Option<&wgpu::Buffer>),
    ) {
        let pixels = source.pixels;
        let size = NonZeroU64::new(u64::from(pixels) * self.output_stride).unwrap();
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("processing parameters"),
            contents: bytemuck::bytes_of(&parameters),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let (nodes, geometry) = match &source.flat_field {
            Some(field) => (
                &field.nodes,
                flat::FieldGeometry::new(
                    field,
                    source.geometry.expect("corrected source geometry"),
                ),
            ),
            None => (
                &self.identity,
                flat::FieldGeometry::uncorrected(source.geometry),
            ),
        };
        let field_uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("processing source geometry"),
            contents: bytemuck::bytes_of(&geometry),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: source.input.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: self.output_binding,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: output,
                    offset: 0,
                    size: Some(size),
                }),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: source.matrix.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: nodes.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: field_uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: locals.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: context.as_entire_binding(),
            },
        ];
        #[cfg(test)]
        let mut entries = entries;
        #[cfg(test)]
        if let Some(prepared) = diagnostics.1 {
            entries.push(wgpu::BindGroupEntry {
                binding: 9,
                resource: prepared.as_entire_binding(),
            });
        }
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("processing pixels"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &entries,
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("inversion/edits"),
            timestamp_writes: {
                #[cfg(test)]
                {
                    diagnostics
                        .0
                        .map(|(query_set, index)| wgpu::ComputePassTimestampWrites {
                            query_set,
                            beginning_of_pass_write_index: Some(index),
                            end_of_pass_write_index: Some(index + 1),
                        })
                }
                #[cfg(not(test))]
                {
                    None
                }
            },
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        let groups = pixels.div_ceil(self.workgroup_size);
        let x = groups.min(device.limits().max_compute_workgroups_per_dimension);
        pass.dispatch_workgroups(x, groups.div_ceil(x), 1);
    }
}

/// Metal resource allocation, including retained native versions; not total process memory.
#[cfg(all(target_os = "macos", any(test, feature = "ui")))]
pub(crate) fn allocated_resource_bytes(device: &wgpu::Device) -> Option<usize> {
    use objc2_metal::MTLDevice;
    // The HAL guard keeps the device alive; this query neither mutates nor destroys resources.
    let native = unsafe { device.as_hal::<wgpu::hal::api::Metal>() }?;
    Some(native.raw_device().currentAllocatedSize())
}

#[cfg(all(test, target_os = "macos"))]
#[path = "gpu_execution_tests.rs"]
mod execution_tests;
#[cfg(test)]
#[path = "gpu_flat_field_tests.rs"]
mod flat_tests;
#[cfg(test)]
#[path = "gpu_tests.rs"]
mod tests;
