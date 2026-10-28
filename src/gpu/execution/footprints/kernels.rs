use super::processing::Processed;
use super::*;
use crate::gpu::ProcessingKernel;
pub(crate) fn shader() -> String {
    [
        ProcessingKernel::kernel_shader(),
        include_str!("../../../shaders/view_geometry.wgsl").to_owned(),
        include_str!("../../../shaders/view_colour.wgsl").to_owned(),
        include_str!("../../../shaders/view_inspection.wgsl").to_owned(),
    ]
    .join("\n")
}

pub(crate) struct Kernels {
    grade: wgpu::ComputePipeline,
    inspection: wgpu::ComputePipeline,
    pub(crate) stages: crate::gpu::pixel_pipeline::Kernels,
    filter: wgpu::ComputePipeline,
    proxy_coverage: wgpu::ComputePipeline,
    copy: wgpu::RenderPipeline,
}
impl Kernels {
    pub async fn new(device: &wgpu::Device) -> Result<Arc<Self>, Error> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let compute = |label, source: String| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: None,
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("filtered photographic grading and inspection"),
            source: wgpu::ShaderSource::Wgsl(shader().into()),
        });
        let photographic = |entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: None,
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let grade = photographic("view_main");
        let inspection = photographic("inspection_main");
        let stages = crate::gpu::pixel_pipeline::Kernels::new(device).await?;
        let filter = compute(
            "original-cell footprint acquisition",
            [
                include_str!("../../../shaders/footprint.wgsl"),
                include_str!("../../../shaders/view_filter.wgsl"),
            ]
            .join("\n"),
        );
        let proxy_coverage = compute(
            "complete camera-proxy coverage",
            include_str!("../../../shaders/view_proxy_coverage.wgsl").to_owned(),
        );
        let copy =
            crate::gpu::raster::render_pipeline(device, "signed/HDR AP1 acquisition copy", COPY);
        if let Some(error) = scope.pop().await {
            return Err(Error::Gpu(error.to_string()));
        }
        Ok(Arc::new(Self {
            grade,
            inspection,
            stages,
            filter,
            proxy_coverage,
            copy,
        }))
    }
    pub(crate) fn grade(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        graded: &Processed,
    ) {
        let Some(grade) = &graded.grade else {
            let view = uniform(device, bytemuck::bytes_of(&graded.view));
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("immutable source-inspection exposure"),
                layout: &self.inspection.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: graded.pixels.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 10,
                        resource: view.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 11,
                        resource: graded.source.pixels.as_entire_binding(),
                    },
                ],
            });
            let groups = ((graded.pixels.size() / 16) as u32).div_ceil(64);
            let x = groups.min(device.limits().max_compute_workgroups_per_dimension);
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.inspection);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(x, groups.div_ceil(x), 1);
            return;
        };
        let working = graded
            .stages
            .input_buffer(device, graded.source.spec.frame().output)
            .expect("validated processing input");
        let pixels = working.as_ref().unwrap_or(&graded.pixels);
        let params = uniform(device, bytemuck::bytes_of(&grade.parameters));
        let view = uniform(device, bytemuck::bytes_of(&graded.view));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("immutable physical-view grade resources"),
            layout: &self.grade.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: pixels.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: grade.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: grade.context.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: view.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: graded.source.pixels.as_entire_binding(),
                },
            ],
        });
        let groups = ((pixels.size() / 16) as u32).div_ceil(64);
        let x = groups.min(device.limits().max_compute_workgroups_per_dimension);
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&self.grade);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(x, groups.div_ceil(x), 1);
        drop(pass);
        self.stages
            .encode(device, encoder, &graded.stages, pixels)
            .expect("validated pixel-stage raster");
        if let Some(working) = &working {
            self.stages.extract(
                device,
                encoder,
                working,
                graded.stages.raster.size(),
                &graded.pixels,
                crate::gpu::pixel_pipeline::OutputRegion {
                    size: graded.plan.output().output,
                    offset: graded
                        .plan
                        .output_offset()
                        .expect("frame-grid output extraction"),
                },
            );
        }
    }
    pub(crate) fn cover_proxy(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        pixels: &wgpu::Buffer,
    ) {
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera-proxy coverage"),
            layout: &self.proxy_coverage.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: pixels.as_entire_binding(),
            }],
        });
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&self.proxy_coverage);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(((pixels.size() / 16) as u32).div_ceil(64), 1, 1);
    }

    pub(crate) fn copy(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        buffer: &wgpu::Buffer,
        texture: &wgpu::Texture,
    ) {
        let size = uniform(
            device,
            bytemuck::cast_slice(&[texture.width(), texture.height(), 0, 0]),
        );
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("neutral AP1 copy resources"),
            layout: &self.copy.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: size.as_entire_binding(),
                },
            ],
        });
        let view = texture.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("neutral AP1 acquisition copy"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.copy);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn filter(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        image: &wgpu::Texture,
        output: &wgpu::Buffer,
        frame: FrameRaster,
        origin: [i32; 2],
        (lo, extent): crate::resample::windows::Window,
    ) -> Result<(), Error> {
        let bounds = frame.processing_bounds()?;
        self.filter_raster(
            device,
            encoder,
            image,
            output,
            [bounds.width, bounds.height],
            frame.output,
            frame.affine_in(bounds)?,
            origin,
            (lo, extent),
        );
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn filter_raster(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        image: &wgpu::Texture,
        output: &wgpu::Buffer,
        source_size: [u32; 2],
        output_size: [u32; 2],
        affine: [[f32; 2]; 3],
        origin: [i32; 2],
        (lo, extent): crate::resample::windows::Window,
    ) {
        let mut bytes = bytemuck::cast_slice(&[
            source_size[0],
            source_size[1],
            output_size[0],
            output_size[1],
            origin[0] as u32,
            origin[1] as u32,
            image.width(),
            image.height(),
        ])
        .to_vec();
        bytes.extend_from_slice(bytemuck::cast_slice(&affine));
        bytes.extend_from_slice(bytemuck::cast_slice(&[0f32; 2]));
        bytes.extend_from_slice(bytemuck::cast_slice(&[lo[0], lo[1], extent[0], extent[1]]));
        let params = uniform(device, &bytes);
        let view = image.create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("physical-view footprint resources"),
            layout: &self.filter.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&self.filter);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(extent[0].div_ceil(8), extent[1].div_ceil(8), 1);
    }
}
pub(crate) fn uniform(device: &wgpu::Device, bytes: &[u8]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytes,
        usage: wgpu::BufferUsages::UNIFORM,
    })
}

pub(crate) fn view_bytes(device: &wgpu::Device, size: [u32; 2]) -> Result<u64, Error> {
    let bytes = u64::from(size[0])
        .checked_mul(u64::from(size[1]))
        .and_then(|n| n.checked_mul(16))
        .ok_or_else(|| Error::Gpu("physical-view size overflow".into()))?;
    if size.contains(&0)
        || size
            .iter()
            .any(|v| *v > device.limits().max_texture_dimension_2d)
        || bytes > device.limits().max_storage_buffer_binding_size
        || bytes > device.limits().max_buffer_size
    {
        return Err(Error::Gpu("physical view exceeds GPU limits".into()));
    }
    Ok(bytes)
}

pub(crate) fn float_buffer(device: &wgpu::Device, size: [u32; 2]) -> Result<wgpu::Buffer, Error> {
    let bytes = view_bytes(device, size)?;
    Ok(device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("physical-view signed/HDR AP1"),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    }))
}

const COPY: &str = r#"
@group(0) @binding(0) var<storage,read> pixels:array<vec4f>;
@group(0) @binding(1) var<uniform> size:vec4u;
@vertex fn vertex(@builtin(vertex_index) i:u32)->@builtin(position) vec4f {
 let p=array<vec2f,3>(vec2f(-1.,-1.),vec2f(3.,-1.),vec2f(-1.,3.))[i];return vec4f(p,0.,1.);
}
@fragment fn fragment(@builtin(position) p:vec4f)->@location(0) vec4f {
 let xy=vec2u(p.xy);return pixels[xy.x+xy.y*size.x];
}"#;
