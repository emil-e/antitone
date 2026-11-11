//! Standalone arithmetic/lifetime control for graph conformance tests.
use super::*;

pub struct Kernel {
    pipeline: wgpu::ComputePipeline,
}

pub struct Dispatch {
    group: wgpu::BindGroup,
    groups: [u32; 2],
    pub check: Arc<Check>,
    _look: Arc<Lut>,
}

impl Kernel {
    pub async fn new(device: &wgpu::Device) -> Result<Self, Error> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ACEScct creative look, tetrahedral"),
            source: wgpu::ShaderSource::Wgsl(super::shader().into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("shared look execution"),
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        if let Some(error) = scope.pop().await {
            return Err(Error::Gpu(error.to_string()));
        }
        Ok(Self { pipeline })
    }

    pub fn bind(
        &self,
        device: &wgpu::Device,
        input: &wgpu::Buffer,
        output: &wgpu::Buffer,
        look: Arc<Lut>,
        count: u32,
        coverage: bool,
    ) -> Result<Dispatch, Error> {
        if count == 0
            || u64::from(count) * 16 > input.size()
            || u64::from(count) * 16 > output.size()
        {
            return Err(Error::Gpu("invalid look dispatch dimensions".into()));
        }
        let groups = count.div_ceil(64);
        let x = groups.min(device.limits().max_compute_workgroups_per_dimension);
        let y = groups.div_ceil(x);
        if y > device.limits().max_compute_workgroups_per_dimension {
            return Err(Error::Gpu("look dispatch exceeds device limits".into()));
        }
        let params = look.parameters(count, x * 64, coverage);
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("look domain and dispatch"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let check = Check::new(device);
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("immutable look dispatch"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: input.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: look.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: check.status.as_entire_binding(),
                },
            ],
        });
        Ok(Dispatch {
            group,
            groups: [x, y],
            check,
            _look: look,
        })
    }

    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder, dispatch: &Dispatch) {
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("creative look before output ACES"),
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &dispatch.group, &[]);
            pass.dispatch_workgroups(dispatch.groups[0], dispatch.groups[1], 1);
        }
        encoder.copy_buffer_to_buffer(&dispatch.check.status, 0, &dispatch.check.readback, 0, 8);
    }
}
