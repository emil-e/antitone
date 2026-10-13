use bytemuck::Zeroable;
use std::{num::NonZeroU64, sync::Arc};

#[path = "gpu_flat_field.rs"]
mod flat;
pub use flat::{GpuFlatField, SourceGeometry};

use glam::{DMat3, DVec3, Vec4, Vec4Swizzles};
use wgpu::util::DeviceExt;

use crate::Error;
use crate::color::{AP1_TO_XYZ, wb_matrix};
use crate::processing::{
    Calibration, Inversion, PhotoSettings, RollSettings, exposure_multiplier, log_abs_expm1,
    log_ratio, validate_camera_matrix,
};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ProcessingParameters {
    dmin_floor: Vec4,
    gamma_mode: Vec4,
    anchor: Vec4,
    post: [Vec4; 3],
    dmin_low: Vec4,
    dmin_tail: Vec4,
    gamma_low: Vec4,
    shape: Vec4,
    anchor_low: Vec4,
    post_low: [Vec4; 3],
    creative: [Vec4; 3],
    creative_low: [Vec4; 3],
}

impl ProcessingParameters {
    pub const BYTE_LEN: usize = std::mem::size_of::<Self>();

    pub fn source() -> Self {
        Self {
            dmin_floor: Vec4::ZERO,
            gamma_mode: Vec4::ZERO,
            anchor: Vec4::ZERO,
            post: matrix_columns(DMat3::IDENTITY),
            ..Self::zeroed()
        }
    }

    pub fn source_inspection(exposure: f64) -> Result<Self, Error> {
        let gain = exposure_multiplier(exposure)?;
        let matrix = DMat3::IDENTITY * gain;
        let mut result = Self {
            post: matrix_columns(matrix),
            ..Self::source()
        };
        result.post_low = std::array::from_fn(|c| {
            let column = [matrix.x_axis, matrix.y_axis, matrix.z_axis][c];
            (column - result.post[c].xyz().as_dvec3())
                .as_vec3()
                .extend(0.)
        });
        Ok(result)
    }

    pub fn positive(
        calibration: Calibration,
        roll: RollSettings,
        photo: PhotoSettings,
        camera_to_ap1: DMat3,
    ) -> Result<Self, Error> {
        validate_camera_matrix(camera_to_ap1)?;
        CameraMatrix::new(camera_to_ap1)?;
        let inversion = Inversion::new(calibration, roll)?;
        photo.validate()?;
        let grade = crate::photographic::Grade::new(photo)?;
        let gain = exposure_multiplier(roll.exposure)? * exposure_multiplier(photo.exposure)?;
        let matrix = wb_matrix(photo.wb)?;
        let (toe, shoulder) = roll.response.shape()?;
        let floor_bound = inversion
            .apply(DVec3::splat(calibration.input_floor))?
            .rgb
            .abs();
        // Above-base continuation is bounded by |expm1(a)| <= 1 for a <= 0.
        let negative_bound = inversion
            .anchor_exponents
            .map(|a| (-log_abs_expm1(a) / toe).exp());
        let largest = floor_bound.max(negative_bound) * gain;
        let working_bound = camera_to_ap1.x_axis.abs() * largest.x
            + camera_to_ap1.y_axis.abs() * largest.y
            + camera_to_ap1.z_axis.abs() * largest.z;
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
                "settings can exceed the f32 working range in response/photographic processing"
                    .into(),
            ));
        }
        let gammas = roll.channel_gammas()?;
        for c in 0..3 {
            let l = log_ratio(calibration.dmin[c], calibration.input_floor);
            let a = toe * l / (gammas[c] - shoulder * l / std::f64::consts::LN_10);
            if !a.is_finite() || a > 4096. {
                return Err(Error::Gpu(
                    "response exponent exceeds the supported GPU precision domain; reduce shoulder"
                        .into(),
                ));
            }
        }
        let reference = split_constants(calibration.dmin, calibration.input_floor)?;
        let gamma = split_constants(gammas, toe)?;
        let logs = split_constants(inversion.anchor_exponents.map(log_abs_expm1), gain.ln())?;
        let k = shoulder as f32;
        let weights = AP1_TO_XYZ.transpose().y_axis;
        let [contrast, colour, luminance] = [
            [photo.contrast, grade.qp, grade.q0, grade.black],
            [
                photo.contrast_colour_mix,
                photo.saturation,
                photo.saturation_density,
                0.,
            ],
            [weights.x, weights.y, weights.z, 0.],
        ]
        .map(|v| split_constants(DVec3::new(v[0], v[1], v[2]), v[3]));
        let controls = [contrast?, colour?, luminance?];
        let mut result = Self {
            dmin_floor: reference[0],
            dmin_low: reference[1],
            dmin_tail: reference[2],
            gamma_mode: gamma[0].xyz().extend(1.),
            gamma_low: gamma[1],
            shape: Vec4::new(toe as f32, k, gamma[1].w, (shoulder - k as f64) as f32),
            anchor: logs[0],
            anchor_low: logs[1],
            post: matrix_columns(matrix),
            creative: std::array::from_fn(|c| controls[c][0]),
            creative_low: std::array::from_fn(|c| controls[c][1] + controls[c][2]),
            ..Self::zeroed()
        };
        result.post_low = std::array::from_fn(|c| {
            let column = [matrix.x_axis, matrix.y_axis, matrix.z_axis][c];
            (column - result.post[c].xyz().as_dvec3())
                .as_vec3()
                .extend(0.)
        });
        if !result.dmin_floor.is_finite()
            || result.dmin_floor.w < f32::MIN_POSITIVE
            || !result.gamma_mode.is_finite()
            || !result.anchor.is_finite()
            || result
                .post
                .iter()
                .chain(result.post_low.iter())
                .any(|v| !v.is_finite())
            || !result.shape.is_finite()
        {
            return Err(Error::Gpu(
                "processing parameters cannot be represented in f32".into(),
            ));
        }
        Ok(result)
    }
}

fn split_constants(value: DVec3, w: f64) -> Result<[Vec4; 3], Error> {
    let values = [value.x, value.y, value.z, w];
    let high = values.map(|v| v as f32);
    let low = std::array::from_fn(|c| (values[c] - high[c] as f64) as f32);
    let tail = std::array::from_fn(|c| (values[c] - high[c] as f64 - low[c] as f64) as f32);
    if (0..4).any(|c| {
        [high[c], low[c], tail[c]]
            .iter()
            .any(|v| !v.is_finite() || (*v != 0. && v.abs() < f32::MIN_POSITIVE))
            || high[c] as f64 + low[c] as f64 + tail[c] as f64 != values[c]
    }) {
        return Err(Error::Gpu(
            "processing constants cannot be represented for precise GPU inversion".into(),
        ));
    }
    Ok([
        Vec4::from_array(high),
        Vec4::from_array(low),
        Vec4::from_array(tail),
    ])
}

fn matrix_columns(matrix: DMat3) -> [Vec4; 3] {
    [matrix.x_axis, matrix.y_axis, matrix.z_axis].map(|v| v.as_vec3().extend(0.0))
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraMatrix {
    high: [Vec4; 3],
    low: [Vec4; 3],
    tail: [Vec4; 3],
}

impl CameraMatrix {
    fn new(matrix: DMat3) -> Result<Self, Error> {
        let columns = [matrix.x_axis, matrix.y_axis, matrix.z_axis];
        let high = columns.map(|v| v.as_vec3().extend(0.));
        let low =
            std::array::from_fn(|c| (columns[c] - high[c].xyz().as_dvec3()).as_vec3().extend(0.));
        let tail = std::array::from_fn(|c| {
            (columns[c] - high[c].xyz().as_dvec3() - low[c].xyz().as_dvec3())
                .as_vec3()
                .extend(0.)
        });
        let result = Self { high, low, tail };
        let bound = matrix.x_axis.abs() + matrix.y_axis.abs() + matrix.z_axis.abs();
        if !bound.is_finite()
            || bound.max_element() > f32::MAX as f64 / 16.
            || columns.iter().any(|v| {
                v.to_array().into_iter().any(|value| {
                    // The smallest nonzero camera code must not underflow a matrix product.
                    value != 0. && value.abs() < f32::MIN_POSITIVE as f64 * 65535.
                })
            })
            || [high, low, tail].into_iter().flatten().any(|v| {
                v.to_array()
                    .into_iter()
                    .any(|value| value != 0. && value.abs() < f32::MIN_POSITIVE)
            })
            || columns.iter().enumerate().any(|(c, v)| {
                !high[c].is_finite()
                    || !low[c].is_finite()
                    || !tail[c].is_finite()
                    || high[c].xyz().as_dvec3() + low[c].xyz().as_dvec3() + tail[c].xyz().as_dvec3()
                        != *v
            })
        {
            return Err(Error::Gpu(
                "camera matrix cannot be represented for GPU conversion".into(),
            ));
        }
        Ok(result)
    }
}

pub struct ProcessingSource {
    input: wgpu::Buffer,
    matrix: wgpu::Buffer,
    pixels: u32,
    camera_to_ap1: DMat3,
    flat_field: Option<(Arc<GpuFlatField>, SourceGeometry)>,
}

impl ProcessingSource {
    pub fn new(device: &wgpu::Device, pixels: &[[u16; 4]], matrix: DMat3) -> Result<Self, Error> {
        let count = u32::try_from(pixels.len())
            .map_err(|_| Error::Gpu("source exceeds GPU limits".into()))?;
        if count == 0
            || u64::from(count) * 8 > device.limits().max_storage_buffer_binding_size
            || u64::from(count) * 16 > device.limits().max_storage_buffer_binding_size
        {
            return Err(Error::Gpu("source exceeds GPU limits".into()));
        }
        validate_camera_matrix(matrix)?;
        let camera_to_ap1 = matrix;
        let matrix = CameraMatrix::new(matrix)?;
        Ok(Self {
            input: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("LibRaw camera RGB16"),
                contents: bytemuck::cast_slice(pixels),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            }),
            matrix: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("camera to AP1 post-inversion constants"),
                contents: bytemuck::bytes_of(&matrix),
                usage: wgpu::BufferUsages::UNIFORM,
            }),
            pixels: count,
            camera_to_ap1,
            flat_field: None,
        })
    }

    pub fn set_flat_field(
        &mut self,
        field: Option<Arc<GpuFlatField>>,
        geometry: SourceGeometry,
    ) -> Result<(), Error> {
        if geometry.pixels() != self.pixels {
            return Err(Error::Gpu(
                "flat-field source geometry differs from RGB16 storage".into(),
            ));
        }
        if let Some(field) = &field {
            field.validate_matrix(self.camera_to_ap1)?;
        }
        self.flat_field = field.map(|field| (field, geometry));
        Ok(())
    }

    pub fn write(&self, queue: &wgpu::Queue, pixels: &[[u16; 4]]) -> Result<(), Error> {
        if pixels.len() != self.pixels as usize {
            return Err(Error::Gpu("invalid replacement source tile".into()));
        }
        queue.write_buffer(&self.input, 0, bytemuck::cast_slice(pixels));
        Ok(())
    }

    pub fn byte_len(&self) -> u64 {
        self.input.size()
    }
}

pub struct ProcessingKernel {
    pipeline: wgpu::ComputePipeline,
    identity: wgpu::Buffer,
}

impl ProcessingKernel {
    pub async fn new(device: &wgpu::Device) -> Result<Self, Error> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("camera RGB16 to AP1 photographic processing"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}\n{}\n{}\n{}",
                    include_str!("shaders/camera_conversion.wgsl"),
                    include_str!("shaders/flat_field.wgsl"),
                    include_str!("shaders/film_response.wgsl"),
                    include_str!("shaders/photographic.wgsl"),
                    include_str!("shaders/processing.wgsl")
                )
                .into(),
            ),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Antitone processing"),
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        if let Some(error) = scope.pop().await {
            return Err(Error::Gpu(error.to_string()));
        }
        Ok(Self {
            pipeline,
            identity: flat::identity_nodes(device),
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
        let pixels = source.pixels;
        let size = NonZeroU64::new(u64::from(pixels) * 16).unwrap();
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("processing parameters"),
            contents: bytemuck::bytes_of(&parameters),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let (nodes, geometry) = match &source.flat_field {
            Some((field, geometry)) => (&field.nodes, flat::FieldGeometry::new(field, *geometry)),
            None => (&self.identity, flat::FieldGeometry::zeroed()),
        };
        let field_uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("flat-field source geometry"),
            contents: bytemuck::bytes_of(&geometry),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("processing pixels"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: source.input.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
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
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("inversion/edits"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        let groups = pixels.div_ceil(64);
        let x = groups.min(device.limits().max_compute_workgroups_per_dimension);
        pass.dispatch_workgroups(x, groups.div_ceil(x), 1);
    }
}

#[cfg(test)]
#[path = "gpu_flat_field_tests.rs"]
mod flat_tests;
#[cfg(test)]
#[path = "gpu_tests.rs"]
mod tests;
