use super::*;

pub(crate) struct Kernels {
    #[cfg(feature = "editor")]
    neutral: wgpu::ComputePipeline,
}

impl Kernels {
    pub async fn new(device: &wgpu::Device, _resources: &CameraResources) -> Result<Self, Error> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("repair component fixture"),
            source: wgpu::ShaderSource::Wgsl(CameraResources::module_shader().into()),
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
            #[cfg(feature = "editor")]
            neutral: pipeline("repair_neutral"),
        };
        if let Some(error) = scope.pop().await {
            return Err(Error::Gpu(error.to_string()));
        }
        Ok(result)
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
