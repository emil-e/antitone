//! Pinned fixed-reference transforms for native built-in baking, not display rendering.
use super::aces;
use crate::{Error, gpu::SubmissionGate};
use std::{borrow::Cow, sync::mpsc, time::Duration};
use wgpu::util::DeviceExt;

pub(crate) struct ReferenceTransforms {
    inverse: wgpu::ComputePipeline,
    forward: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    tables: [wgpu::TextureView; 2],
    sampler: wgpu::Sampler,
    hues: wgpu::Buffer,
}

impl ReferenceTransforms {
    pub(crate) async fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Self, Error> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let buffer = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            count: None,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
        };
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            count: None,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fixed p3rec50 reference resources"),
            entries: &[
                buffer(0, true),
                texture(1),
                texture(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    count: None,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                },
                buffer(4, false),
                buffer(5, true),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fixed p3rec50 reference"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let mut shared_hues = None;
        let mut pipelines = Vec::new();
        for (name, pinned) in [
            (
                "inverse",
                include_str!("../assets/ocio/looks-reference/inverse.glsl"),
            ),
            (
                "forward",
                include_str!("../assets/ocio/looks-reference/forward.glsl"),
            ),
        ] {
            let (body, hues) = aces::adapt(pinned)?;
            if shared_hues.as_ref().is_some_and(|other| other != &hues) {
                return Err(Error::Gpu("fixed-reference hue resources differ".into()));
            }
            shared_hues = Some(hues);
            // The pinned tables have one mip level. Explicit LOD is resource
            // glue for compute, where implicit fragment derivatives do not exist.
            let source = format!(
                "#version 450\nlayout(local_size_x=64) in;\nlayout(set=0,binding=3) uniform sampler aces_sampler;\n#define texture(s,uv) textureLod(s,uv,0.0)\n{body}\nlayout(set=0,binding=0,std430) readonly buffer Inputs {{ vec4 inputs[]; }};\nlayout(set=0,binding=4,std430) buffer Outputs {{ vec4 outputs[]; }};\nvoid main() {{ uint i=gl_GlobalInvocationID.x; if (i<uint(inputs.length())) outputs[i]=look_reference(inputs[i]); }}"
            );
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(name),
                source: wgpu::ShaderSource::Glsl {
                    shader: Cow::Owned(source),
                    stage: wgpu::naga::ShaderStage::Compute,
                    defines: Default::default(),
                },
            });
            pipelines.push(
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(name),
                    layout: Some(&pipeline_layout),
                    module: &module,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    cache: None,
                }),
            );
        }
        let hues = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fixed-reference cusp hues"),
            contents: bytemuck::cast_slice(&shared_hues.unwrap()),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let mut tables = Vec::new();
        for (index, bytes) in [
            include_bytes!("../assets/ocio/looks-reference/reach.f32").as_slice(),
            include_bytes!("../assets/ocio/looks-reference/cusp.f32").as_slice(),
        ]
        .into_iter()
        .enumerate()
        {
            let channels = if index == 0 { 1 } else { 3 };
            if bytes.len() != 363 * channels * 4 {
                return Err(Error::Gpu(
                    "unexpected fixed-reference table dimensions".into(),
                ));
            }
            let mut values = Vec::with_capacity(363 * 4);
            for pixel in bytes.chunks_exact(channels * 4) {
                let mut value = [0., 0., 0., 1.];
                for c in 0..channels {
                    value[c] = f32::from_le_bytes(pixel[c * 4..c * 4 + 4].try_into().unwrap());
                    if !value[c].is_finite() {
                        return Err(Error::Gpu("nonfinite fixed-reference table".into()));
                    }
                }
                values.extend(value);
            }
            let table = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("fixed-reference f32 table"),
                size: wgpu::Extent3d {
                    width: 363,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                table.as_image_copy(),
                bytemuck::cast_slice(&values),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(363 * 16),
                    rows_per_image: Some(1),
                },
                table.size(),
            );
            tables.push(table.create_view(&Default::default()));
        }
        let sampler = device.create_sampler(&Default::default());
        if let Some(error) = scope.pop().await {
            return Err(Error::Gpu(error.to_string()));
        }
        let forward = pipelines.pop().unwrap();
        let inverse = pipelines.pop().unwrap();
        Ok(Self {
            inverse,
            forward,
            layout,
            tables: tables.try_into().unwrap(),
            sampler,
            hues,
        })
    }

    /// Blocking readback is worker-only, outside the shared host submission gate.
    pub(crate) fn apply(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        inverse: bool,
        input: &[[f32; 4]],
    ) -> Result<Vec<[f32; 4]>, Error> {
        if input.is_empty()
            || input.len() > usize::try_from(crate::looks::MAX_GRID_SIZE.pow(3)).unwrap()
            || input.iter().flatten().any(|v| !v.is_finite())
        {
            return Err(Error::Gpu("invalid fixed-reference batch".into()));
        }
        let bytes = std::mem::size_of_val(input) as u64;
        if bytes > device.limits().max_storage_buffer_binding_size
            || bytes > device.limits().max_buffer_size
        {
            return Err(Error::Gpu(
                "fixed-reference batch exceeds device limits".into(),
            ));
        }
        let source = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fixed-reference input"),
            contents: bytemuck::cast_slice(input),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fixed-reference output"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fixed-reference readback"),
            size: bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fixed-reference transform"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: source.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.tables[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.tables[1]),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: output.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: self.hues.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fixed-reference transform"),
                ..Default::default()
            });
            pass.set_pipeline(if inverse {
                &self.inverse
            } else {
                &self.forward
            });
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups((input.len() as u32).div_ceil(64), 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
        let submission = submissions.submit(queue, [encoder.finish()]);
        let (send, receive) = mpsc::channel();
        readback.map_async(wgpu::MapMode::Read, .., move |result| {
            let _ = send.send(result);
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(Duration::from_secs(30)),
            })
            .map_err(|e| Error::Gpu(e.to_string()))?;
        receive
            .recv_timeout(Duration::from_secs(30))
            .map_err(|e| Error::Gpu(e.to_string()))?
            .map_err(|e| Error::Gpu(e.to_string()))?;
        let mapped = readback
            .get_mapped_range(..)
            .map_err(|e| Error::Gpu(e.to_string()))?;
        let result = bytemuck::cast_slice::<u8, [f32; 4]>(&mapped).to_vec();
        drop(mapped);
        readback.unmap();
        if result.iter().flatten().any(|v| !v.is_finite()) {
            return Err(Error::Gpu("nonfinite fixed-reference result".into()));
        }
        Ok(result)
    }
}
