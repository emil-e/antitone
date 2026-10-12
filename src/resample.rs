use wgpu::util::DeviceExt;

const SHADER: &str = r#"
struct Parameters { source:vec2u, output:vec2u, origin:vec2u, tile:vec2u }
@group(0) @binding(0) var image:texture_2d<f32>;
@group(0) @binding(1) var<storage,read_write> sums:array<vec4f>;
@group(0) @binding(2) var<uniform> p:Parameters;
@compute @workgroup_size(8,8) fn main(@builtin(global_invocation_id) id:vec3u) {
 if any(id.xy>=p.output) { return; }
 let scale=vec2f(p.source)/vec2f(p.output);
 let start=vec2f(id.xy)*scale;
 let end=vec2f(id.xy+vec2u(1))*scale;
 let lo=max(vec2u(floor(start)),p.origin);
 let hi=min(min(vec2u(ceil(end)),p.origin+p.tile),p.source);
 if any(lo>=hi) {return;}
 var sum=vec3f(0.);
 for(var y=lo.y;y<hi.y;y++) {
  let wy=max(0.,min(end.y,f32(y+1))-max(start.y,f32(y)));
  for(var x=lo.x;x<hi.x;x++) {
   let wx=max(0.,min(end.x,f32(x+1))-max(start.x,f32(x)));
   sum+=textureLoad(image,vec2i(vec2u(x,y)-p.origin),0).rgb*wx*wy;
  }
 }
 let index=id.x+id.y*p.output.x;
 sums[index]+=vec4f(sum/(scale.x*scale.y),0.);
}
"#;
const FINISH: &str = r#"
@group(0) @binding(0) var<storage,read> sums:array<vec4f>;
@group(0) @binding(1) var<uniform> size:vec4u;
@vertex fn vertex(@builtin(vertex_index) i:u32)->@builtin(position) vec4f {
 let p=array<vec2f,3>(vec2f(-1.,-1.),vec2f(3.,-1.),vec2f(-1.,3.))[i];return vec4f(p,0.,1.);
}
@fragment fn fragment(@builtin(position) position:vec4f)->@location(0) vec4f {
 let xy=vec2u(position.xy);return vec4f(sums[xy.x+xy.y*size.x].rgb,1.);
}
"#;

pub struct AreaAverage {
    compute: wgpu::ComputePipeline,
    finish: wgpu::RenderPipeline,
    sums: wgpu::Buffer,
    dimensions: [u32; 4],
    affine: Option<[[f32; 2]; 3]>,
}
impl AreaAverage {
    pub async fn new(
        device: &wgpu::Device,
        source: [u32; 2],
        output: [u32; 2],
    ) -> Result<Self, crate::Error> {
        Self::create(device, source, output, None).await
    }
    pub async fn new_affine(
        device: &wgpu::Device,
        raster: crate::frame_raster::FrameRaster,
    ) -> Result<Self, crate::Error> {
        let area = raster.bounds()?;
        Self::create(
            device,
            [area.width, area.height],
            raster.output,
            Some(raster.affine()?),
        )
        .await
    }
    async fn create(
        device: &wgpu::Device,
        source: [u32; 2],
        output: [u32; 2],
        affine: Option<[[f32; 2]; 3]>,
    ) -> Result<Self, crate::Error> {
        let size = (u64::from(output[0]) * u64::from(output[1]))
            .checked_mul(16)
            .ok_or_else(|| crate::Error::Gpu("area-average size overflow".into()))?;
        if source.contains(&0)
            || output.contains(&0)
            || (affine.is_none() && (output[0] > source[0] || output[1] > source[1]))
            || size > device.limits().max_storage_buffer_binding_size
        {
            return Err(crate::Error::Gpu(
                "invalid area-average dimensions or GPU budget".into(),
            ));
        }
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("post-render exact-area averaging"),
            source: wgpu::ShaderSource::Wgsl(
                if affine.is_some() {
                    include_str!("shaders/affine_area.wgsl")
                } else {
                    SHADER
                }
                .into(),
            ),
        });
        let compute = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("settled preview reconstruction"),
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let sums = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("linear target RGB area sums"),
            size,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("settled image copy"),
            source: wgpu::ShaderSource::Wgsl(FINISH.into()),
        });
        let finish = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("settled image finish"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba32Float,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        if let Some(error) = scope.pop().await {
            return Err(crate::Error::Gpu(error.to_string()));
        }
        Ok(Self {
            compute,
            finish,
            sums,
            dimensions: [source[0], source[1], output[0], output[1]],
            affine,
        })
    }
    pub fn accumulate(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        tile: &wgpu::Texture,
        origin: [u32; 2],
    ) {
        let parameters = [
            self.dimensions[0],
            self.dimensions[1],
            self.dimensions[2],
            self.dimensions[3],
            origin[0],
            origin[1],
            tile.width(),
            tile.height(),
        ];
        let mut bytes = bytemuck::cast_slice(&parameters).to_vec();
        if let Some(affine) = self.affine {
            bytes.extend_from_slice(bytemuck::cast_slice(&affine));
            bytes.extend_from_slice(bytemuck::cast_slice(&[0_f32; 2]));
        }
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: &bytes,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let view = tile.create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.compute.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.sums.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&self.compute);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(
            self.dimensions[2].div_ceil(8),
            self.dimensions[3].div_ceil(8),
            1,
        );
    }
    pub fn finish(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        output: &wgpu::Texture,
    ) {
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&[self.dimensions[2], self.dimensions[3], 0, 0]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.finish.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.sums.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        let view = output.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("settled full-source-derived preview"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.finish);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
}
