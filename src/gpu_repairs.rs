//! Immutable sparse neutral-AP1 repairs; originals and unmodified view sources remain untouched.
use super::*;
#[cfg(feature = "editor")]
use crate::frame_raster::FrameRaster;
use crate::{dust_repair::EvaluatedRepair, geometry::SourceRegion};
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
    buffer: Option<wgpu::Buffer>,
    region: SourceRegion,
    bound: DVec3,
}

impl Patch {
    #[cfg(any(feature = "editor", test))]
    pub fn new(device: &wgpu::Device, evaluated: Arc<EvaluatedRepair>) -> Result<Arc<Self>, Error> {
        let mut patch = Self::logical(evaluated)?;
        patch.buffer = patch.buffer_for(device, patch.region)?;
        Ok(Arc::new(patch))
    }

    fn logical(evaluated: Arc<EvaluatedRepair>) -> Result<Self, Error> {
        let support = evaluated.support();
        let [x, y] = support.origin();
        let [width, height] = support.size();
        let mut bound = DVec3::ZERO;
        for (_, value) in evaluated.pixels() {
            if value.iter().any(|value| !value.is_finite()) {
                return Err(Error::Gpu("invalid neutral repair values".into()));
            }
            bound = bound.max(DVec3::from_array(value.map(f64::from)).abs());
        }
        Ok(Self {
            evaluated,
            buffer: None,
            region: SourceRegion {
                x,
                y,
                width,
                height,
            },
            bound,
        })
    }

    fn buffer_for(
        &self,
        device: &wgpu::Device,
        region: SourceRegion,
    ) -> Result<Option<wgpu::Buffer>, Error> {
        if let Some(buffer) = &self.buffer {
            return Ok(Some(buffer.clone()));
        }
        let evaluated = &self.evaluated;
        let support = evaluated.support();
        let [x, y] = support.origin();
        let [width, _] = support.size();
        let pixels = evaluated
            .pixels()
            .filter(|(point, _)| {
                point[0] >= region.x
                    && point[1] >= region.y
                    && point[0] < region.x + region.width
                    && point[1] < region.y + region.height
            })
            .map(|(point, value)| Pixel {
                position: [point[0], point[1], (point[1] - y) * width + point[0] - x, 0],
                value: [value[0], value[1], value[2], 0.],
            })
            .collect::<Vec<_>>();
        if pixels.is_empty() {
            return Ok(None);
        }
        let bytes = bytemuck::cast_slice(&pixels);
        if bytes.len() as u64 > device.limits().max_buffer_size
            || bytes.len() as u64 > device.limits().max_storage_buffer_binding_size
        {
            return Err(Error::Gpu("repair patch exceeds GPU limits".into()));
        }
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("immutable sparse neutral AP1 repair"),
            contents: bytes,
            usage: wgpu::BufferUsages::STORAGE,
        });
        Ok(Some(buffer))
    }
}

#[derive(Clone, Default)]
pub(crate) struct Snapshot(pub Vec<Arc<Patch>>);

impl Snapshot {
    /// Offscreen consumers retain logical sparse content and upload only each demanded original unit.
    pub(crate) fn logical(evaluated: Vec<Arc<EvaluatedRepair>>) -> Result<Self, Error> {
        evaluated
            .into_iter()
            .map(|repair| Patch::logical(repair).map(Arc::new))
            .collect::<Result<Vec<_>, _>>()
            .map(Self)
    }

    #[cfg(test)]
    pub(crate) fn from_evaluations(
        device: &wgpu::Device,
        evaluated: Vec<Arc<EvaluatedRepair>>,
    ) -> Result<Self, Error> {
        evaluated
            .into_iter()
            .map(|repair| Patch::new(device, repair))
            .collect::<Result<Vec<_>, _>>()
            .map(Self)
    }

    #[cfg(all(feature = "raw", feature = "export"))]
    pub(crate) fn require_area(&self, area: SourceRegion) -> Result<(), Error> {
        for (index, patch) in self.0.iter().enumerate() {
            if !area.contains_region(patch.region)
                || self.0[..index].iter().any(|other| {
                    other
                        .evaluated
                        .support()
                        .intersects(patch.evaluated.support())
                })
            {
                return Err(Error::Gpu(
                    "repair snapshot has overlapping or off-source support".into(),
                ));
            }
        }
        Ok(())
    }

    #[cfg(feature = "editor")]
    pub(crate) fn byte_len(&self) -> u64 {
        (std::mem::size_of::<Self>() + self.0.capacity() * std::mem::size_of::<Arc<Patch>>()) as u64
            + self
                .0
                .iter()
                .map(|patch| {
                    patch.buffer.as_ref().map_or(0, wgpu::Buffer::size)
                        + patch.evaluated.retained_bytes() as u64
                        + std::mem::size_of::<Patch>() as u64
                })
                .sum::<u64>()
    }

    #[cfg(feature = "editor")]
    pub(crate) fn estimate(repairs: &[crate::dust_repair::Repair]) -> Option<u64> {
        repairs
            .iter()
            .try_fold(std::mem::size_of::<Self>() as u64, |total, repair| {
                let size = repair.support.size();
                let values = u64::from(size[0])
                    .checked_mul(u64::from(size[1]))?
                    .checked_mul(std::mem::size_of::<crate::dust_repair::Ap1>() as u64)?;
                let donors = (repair.support.marked_pixels().max(4) as u64)
                    .checked_mul(2)?
                    .checked_mul(std::mem::size_of::<crate::dust_repair::DonorMatch>() as u64)?;
                total
                    .checked_add(values)?
                    .checked_add(donors)?
                    .checked_add(repair.support.retained_bytes() as u64)?
                    .checked_add(
                        (std::mem::size_of::<EvaluatedRepair>()
                            + std::mem::size_of::<Patch>()
                            + 3 * std::mem::size_of::<Arc<Patch>>()) as u64,
                    )
            })
    }

    pub fn bound(&self) -> DVec3 {
        self.0
            .iter()
            .fold(DVec3::ZERO, |bound, patch| bound.max(patch.bound))
    }
}

impl PartialEq for Snapshot {
    fn eq(&self, other: &Self) -> bool {
        self.0.len() == other.0.len()
            && self.0.iter().zip(&other.0).all(|(a, b)| {
                Arc::ptr_eq(a, b)
                    || a.evaluated.support() == b.evaluated.support()
                        && a.evaluated
                            .pixels()
                            .zip(b.evaluated.pixels())
                            .all(|(a, b)| {
                                a.0 == b.0 && a.1.map(f32::to_bits) == b.1.map(f32::to_bits)
                            })
            })
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
    #[cfg(feature = "editor")]
    neutral: wgpu::ComputePipeline,
    #[cfg(feature = "editor")]
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
            #[cfg(feature = "editor")]
            neutral: pipeline("repair_neutral"),
            #[cfg(feature = "editor")]
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
    ) -> Result<(), Error> {
        if input.parameters().gamma_mode.w <= 0.5 {
            return Ok(());
        }
        encode_original(
            &self.original,
            device,
            encoder,
            kernel,
            output,
            input,
            geometry,
        )
    }

    #[cfg(feature = "editor")]
    pub fn neutral(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        output: &wgpu::Buffer,
        repairs: &Snapshot,
        geometry: SourceGeometry,
    ) -> Result<(), Error> {
        let region = geometry.region();
        let size = geometry.size();
        let tile = uniform(
            device,
            bytemuck::cast_slice(&[region.x, region.y, size[0], size[1], 0, 0, 0, 0]),
        );
        for patch in &repairs.0 {
            if !intersects(patch.region, region) {
                continue;
            }
            let Some(buffer) = patch.buffer_for(device, region)? else {
                continue;
            };
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("absolute ungraded AP1 repair values"),
                layout: &self.neutral.get_bind_group_layout(0),
                entries: &[entry(1, output), entry(12, &buffer), entry(13, &tile)],
            });
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.neutral);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups((buffer.size() / 32).div_ceil(64) as u32, 1, 1);
        }
        Ok(())
    }

    /// Copy only physical footprints touched by exact support, preserving every other bit.
    #[cfg(feature = "editor")]
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
            let Some(buffer) = patch.buffer_for(device, patch.region)? else {
                continue;
            };
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("exact support footprint copy"),
                layout: &self.copy.get_bind_group_layout(0),
                entries: &[
                    entry(1, output),
                    entry(12, &buffer),
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

#[cfg(all(feature = "raw", feature = "export"))]
pub(crate) struct ExportKernel(wgpu::ComputePipeline);

#[cfg(all(feature = "raw", feature = "export"))]
impl ExportKernel {
    pub(crate) async fn new(device: &wgpu::Device) -> Result<Self, Error> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let shader = [
            ProcessingKernel::pipeline_shader(true, true),
            include_str!("shaders/footprint.wgsl").into(),
            include_str!("shaders/repairs.wgsl").into(),
            include_str!("shaders/repairs_export.wgsl").into(),
        ]
        .join("\n");
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("compensated neutral repair grading"),
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("repair_export"),
            layout: None,
            module: &module,
            entry_point: Some("repair_export"),
            compilation_options: Default::default(),
            cache: None,
        });
        if let Some(error) = scope.pop().await {
            return Err(Error::Gpu(error.to_string()));
        }
        Ok(Self(pipeline))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        kernel: &ProcessingKernel,
        output: &wgpu::Buffer,
        input: &ProcessingInput,
        geometry: SourceGeometry,
    ) -> Result<(), Error> {
        encode_original(&self.0, device, encoder, kernel, output, input, geometry)
    }
}

#[allow(clippy::too_many_arguments)]
fn encode_original(
    pipeline: &wgpu::ComputePipeline,
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    kernel: &ProcessingKernel,
    output: &wgpu::Buffer,
    input: &ProcessingInput,
    geometry: SourceGeometry,
) -> Result<(), Error> {
    let parameters = uniform(device, bytemuck::bytes_of(&input.parameters()));
    let field = uniform(
        device,
        bytemuck::bytes_of(&flat::FieldGeometry::uncorrected(Some(geometry))),
    );
    let (locals, context) = input
        .grade()
        .map_or((&kernel.empty_locals, &kernel.empty_local_context), |g| {
            (&g.buffer, &g.context)
        });
    let mut offset = 0;
    for region in geometry.regions() {
        let tile = uniform(
            device,
            bytemuck::cast_slice(&[
                region.x,
                region.y,
                region.width,
                region.height,
                offset,
                0,
                0,
                0,
            ]),
        );
        for patch in &input.repairs.0 {
            if !intersects(patch.region, region) {
                continue;
            }
            let Some(buffer) = patch.buffer_for(device, region)? else {
                continue;
            };
            let binding = if kernel.output_stride == 48 { 10 } else { 1 };
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("source-coordinate repair grading resources"),
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[
                    entry(binding, output),
                    entry(2, &parameters),
                    entry(5, &field),
                    entry(6, locals),
                    entry(7, context),
                    entry(12, &buffer),
                    entry(13, &tile),
                ],
            });
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups((buffer.size() / 32).div_ceil(64) as u32, 1, 1);
        }
        offset += region.width * region.height;
    }
    Ok(())
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
