//! Immutable sparse neutral-AP1 repairs; originals and unmodified view sources remain untouched.
use super::*;
use crate::{dust_repair::EvaluatedRepair, frame_raster::FrameRaster, geometry::SourceRegion};
use std::sync::Arc;
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Pixel {
    position: [u32; 4],
    value: [f32; 4],
}

pub(crate) struct Patch {
    pub evaluated: Arc<EvaluatedRepair>,
    buffer: wgpu::Buffer,
    region: SourceRegion,
    bound: DVec3,
}
impl Patch {
    pub fn new(device: &wgpu::Device, evaluated: Arc<EvaluatedRepair>) -> Result<Arc<Self>, Error> {
        let support = evaluated.support();
        let [x, y] = support.origin();
        let [width, height] = support.size();
        let pixels = evaluated
            .pixels()
            .map(|(point, value)| Pixel {
                position: [point[0], point[1], (point[1] - y) * width + point[0] - x, 0],
                value: [value[0], value[1], value[2], 0.],
            })
            .collect::<Vec<_>>();
        if pixels.is_empty()
            || pixels
                .iter()
                .any(|p| p.value.iter().any(|v| !v.is_finite()))
        {
            return Err(Error::Gpu("invalid neutral repair values".into()));
        }
        let bytes = bytemuck::cast_slice(&pixels);
        if bytes.len() as u64 > device.limits().max_buffer_size
            || bytes.len() as u64 > device.limits().max_storage_buffer_binding_size
        {
            return Err(Error::Gpu("repair patch exceeds GPU limits".into()));
        }
        let bound = pixels.iter().fold(DVec3::ZERO, |bound, p| {
            bound.max(DVec3::new(p.value[0] as f64, p.value[1] as f64, p.value[2] as f64).abs())
        });
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("immutable sparse neutral AP1 repair"),
            contents: bytes,
            usage: wgpu::BufferUsages::STORAGE,
        });
        Ok(Arc::new(Self {
            evaluated,
            buffer,
            region: SourceRegion {
                x,
                y,
                width,
                height,
            },
            bound,
        }))
    }
}

#[derive(Clone, Default)]
pub(crate) struct Snapshot(pub Vec<Arc<Patch>>);
impl Snapshot {
    pub fn bound(&self) -> DVec3 {
        self.0
            .iter()
            .fold(DVec3::ZERO, |bound, patch| bound.max(patch.bound))
    }
}
impl PartialEq for Snapshot {
    fn eq(&self, other: &Self) -> bool {
        self.0.len() == other.0.len() && self.0.iter().zip(&other.0).all(|(a, b)| Arc::ptr_eq(a, b))
    }
}
impl std::fmt::Debug for Snapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("NeutralRepairs")
            .field(&self.0.len())
            .finish()
    }
}

pub(crate) struct Kernels {
    original: wgpu::ComputePipeline,
    neutral: wgpu::ComputePipeline,
    copy: wgpu::ComputePipeline,
}
impl Kernels {
    pub async fn new(device: &wgpu::Device) -> Result<Self, Error> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let shader = [
            ProcessingKernel::shader(true),
            include_str!("shaders/footprint.wgsl").into(),
            include_str!("shaders/repairs.wgsl").into(),
        ]
        .join("\n");
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("source-coordinate neutral AP1 repair composition"),
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });
        let pipeline = |entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: None,
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let result = Self {
            original: pipeline("repair_original"),
            neutral: pipeline("repair_neutral"),
            copy: pipeline("repair_copy"),
        };
        if let Some(error) = scope.pop().await {
            return Err(Error::Gpu(error.to_string()));
        }
        Ok(result)
    }

    /// Only support pixels are overwritten; the normal precise original pipeline is untouched elsewhere.
    pub fn original(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        kernel: &ProcessingKernel,
        output: &wgpu::Buffer,
        input: &ProcessingInput,
        geometry: SourceGeometry,
    ) {
        if input.parameters().gamma_mode.w <= 0.5 {
            return;
        }
        let region = geometry.region();
        let size = geometry.size();
        let parameters = uniform(device, bytemuck::bytes_of(&input.parameters()));
        let field = uniform(
            device,
            bytemuck::bytes_of(&flat::FieldGeometry::uncorrected(Some(geometry))),
        );
        let tile = uniform(
            device,
            bytemuck::cast_slice(&[region.x, region.y, size[0], size[1]]),
        );
        let (locals, context) = input
            .grade()
            .map_or((&kernel.empty_locals, &kernel.empty_local_context), |g| {
                (&g.buffer, &g.context)
            });
        for patch in &input.repairs.0 {
            if !intersects(patch.region, region) {
                continue;
            }
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("original neutral repair grading resources"),
                layout: &self.original.get_bind_group_layout(0),
                entries: &[
                    entry(1, output),
                    entry(2, &parameters),
                    entry(5, &field),
                    entry(6, locals),
                    entry(7, context),
                    entry(12, &patch.buffer),
                    entry(13, &tile),
                ],
            });
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.original);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups((patch.buffer.size() / 32).div_ceil(64) as u32, 1, 1);
        }
    }

    pub fn neutral(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        output: &wgpu::Buffer,
        repairs: &Snapshot,
        geometry: SourceGeometry,
    ) {
        let region = geometry.region();
        let size = geometry.size();
        let tile = uniform(
            device,
            bytemuck::cast_slice(&[region.x, region.y, size[0], size[1]]),
        );
        for patch in &repairs.0 {
            if !intersects(patch.region, region) {
                continue;
            }
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("absolute ungraded AP1 repair values"),
                layout: &self.neutral.get_bind_group_layout(0),
                entries: &[entry(1, output), entry(12, &patch.buffer), entry(13, &tile)],
            });
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.neutral);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups((patch.buffer.size() / 32).div_ceil(64) as u32, 1, 1);
        }
    }

    /// Copy only physical footprints touched by exact support, preserving every other bit.
    #[allow(clippy::too_many_arguments)]
    pub fn copy_regions(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        output: &wgpu::Buffer,
        source: (&wgpu::Buffer, ([u32; 2], [u32; 2])),
        frame: FrameRaster,
        window: ([u32; 2], [u32; 2]),
        repairs: &Snapshot,
    ) -> Result<(), Error> {
        let bounds = frame.bounds()?;
        let affine = frame.affine()?;
        for patch in &repairs.0 {
            let r = patch.region;
            if !intersects(r, bounds) {
                continue;
            }
            let mut bytes = bytemuck::cast_slice(&[
                bounds.x,
                bounds.y,
                frame.output[0],
                frame.output[1],
                r.x,
                r.y,
                r.width,
                r.height,
                window.0[0],
                window.0[1],
                window.1[0],
                window.1[1],
                (source.1).0[0],
                (source.1).0[1],
                (source.1).1[0],
                (source.1).1[1],
            ])
            .to_vec();
            for pair in affine {
                bytes.extend_from_slice(bytemuck::cast_slice(&[pair[0], pair[1], 0., 0.]));
            }
            let view = uniform(device, &bytes);
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("exact support footprint copy"),
                layout: &self.copy.get_bind_group_layout(0),
                entries: &[
                    entry(1, output),
                    entry(12, &patch.buffer),
                    entry(14, &view),
                    entry(15, source.0),
                ],
            });
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.copy);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(window.1[0].div_ceil(8), window.1[1].div_ceil(8), 1);
        }
        Ok(())
    }
}
fn uniform(device: &wgpu::Device, bytes: &[u8]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytes,
        usage: wgpu::BufferUsages::UNIFORM,
    })
}
fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}
fn intersects(a: SourceRegion, b: SourceRegion) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}
