//! Shared creative-LUT execution with finite-input clipping and nonfinite rejection.
use crate::{
    Error,
    looks::{LookId, Lut3D},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use wgpu::util::DeviceExt;

pub struct Kernel {
    pipeline: wgpu::ComputePipeline,
    validation: wgpu::ComputePipeline,
}

pub struct Lut {
    buffer: wgpu::Buffer,
    minimum: [f32; 3],
    maximum: [f32; 3],
    size: u32,
    id: LookId,
}

impl Lut {
    /// Allocation-free admission for selection and candidate documents, before acceptance.
    pub fn require_limits(limits: &wgpu::Limits, look: &Lut3D) -> Result<(), Error> {
        let bytes = look.nodes().len() as u64 * 16;
        if bytes > limits.max_storage_buffer_binding_size || bytes > limits.max_buffer_size {
            return Err(Error::Gpu(
                "look exceeds this device's resource limits".into(),
            ));
        }
        Ok(())
    }

    pub fn new(device: &wgpu::Device, look: &Lut3D) -> Result<Arc<Self>, Error> {
        Self::require_limits(&device.limits(), look)?;
        let nodes = look
            .nodes()
            .iter()
            .map(|p| [p[0], p[1], p[2], 0.])
            .collect::<Vec<_>>();
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("immutable ACEScct/AP1 look samples"),
            contents: bytemuck::cast_slice(&nodes),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let (minimum, maximum) = look.domain();
        Ok(Arc::new(Self {
            buffer,
            minimum,
            maximum,
            size: look.size(),
            id: look.id(),
        }))
    }

    pub fn id(&self) -> LookId {
        self.id
    }

    pub fn byte_len(&self) -> u64 {
        self.buffer.size()
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Parameters {
    minimum: [f32; 4],
    maximum: [f32; 4],
    reciprocal: [f32; 4],
    dimensions: [u32; 4],
}

/// Fresh for each dispatch; mapped/retained old checks cannot alias a new render.
pub struct Check {
    status: wgpu::Buffer,
    readback: wgpu::Buffer,
    started: AtomicBool,
    result: Arc<Mutex<Option<Result<(), Error>>>>,
}

impl Check {
    fn new(device: &wgpu::Device) -> Arc<Self> {
        Arc::new(Self {
            status: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("look admission status"),
                contents: bytemuck::cast_slice(&[0u32, u32::MAX]),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            }),
            readback: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("look admission readback"),
                size: 8,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
            started: AtomicBool::new(false),
            result: Arc::new(Mutex::new(None)),
        })
    }

    /// Start mapping only after the encoder containing the check has been submitted.
    pub fn begin(&self, ready: impl FnOnce() + Send + 'static) {
        if self.started.swap(true, Ordering::AcqRel) {
            return;
        }
        let buffer = self.readback.clone();
        let state = self.result.clone();
        self.readback
            .map_async(wgpu::MapMode::Read, .., move |mapped| {
                let result = match mapped {
                    Ok(()) => Self::read_result(&buffer),
                    Err(error) => Err(Error::Gpu(error.to_string())),
                };

                *state.lock().unwrap() = Some(result);
                ready();
            });
    }

    fn read_result(buffer: &wgpu::Buffer) -> Result<(), Error> {
        let status = buffer
            .get_mapped_range(..)
            .map(|view| {
                let values = bytemuck::cast_slice::<u8, u32>(&view);
                (values[0], values[1])
            })
            .map_err(|error| Error::Gpu(error.to_string()));
        buffer.unmap();

        let (flags, first) = status?;
        let problem = match flags {
            0 => return Ok(()),
            1 => "received a nonfinite scene-linear input",
            2 => "produced a nonfinite scene-linear value",
            _ => "encountered nonfinite input and output",
        };
        Err(Error::Rendering(format!(
            "The selected look {problem} (first rejected sample {first})."
        )))
    }

    pub fn result(&self) -> Option<Result<(), Error>> {
        self.result.lock().unwrap().clone()
    }

    pub fn pending(&self) -> bool {
        self.result.lock().unwrap().is_none()
    }
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
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/look.wgsl").into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("shared look execution"),
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let validation_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rendered look admission"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}",
                    include_str!("../shaders/footprint.wgsl"),
                    include_str!("../shaders/look_validation.wgsl")
                )
                .into(),
            ),
        });
        let validation = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("rendered look admission"),
            layout: None,
            module: &validation_shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        if let Some(error) = scope.pop().await {
            return Err(Error::Gpu(error.to_string()));
        }
        Ok(Self {
            pipeline,
            validation,
        })
    }

    pub fn check_rendered(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        image: &wgpu::Texture,
    ) -> Arc<Check> {
        self.check_projected(
            device,
            encoder,
            image,
            crate::geometry::TextureMapping::IDENTITY,
        )
        .expect("a nonempty texture has a finite identity footprint")
    }

    /// Check only texels with positive contribution to the requested source footprint.
    pub fn check_projected(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        image: &wgpu::Texture,
        mapping: crate::geometry::TextureMapping,
    ) -> Result<Arc<Check>, Error> {
        let size = [image.width() as f32, image.height() as f32];
        let values = [
            mapping.origin[0] * size[0],
            mapping.origin[1] * size[1],
            mapping.x_axis[0] * size[0],
            mapping.x_axis[1] * size[1],
            mapping.y_axis[0] * size[0],
            mapping.y_axis[1] * size[1],
            0.,
            0.,
        ];
        let determinant = values[2] * values[5] - values[3] * values[4];
        if values.iter().any(|value| !value.is_finite())
            || !determinant.is_finite()
            || determinant == 0.
        {
            return Err(Error::Gpu("invalid look admission footprint".into()));
        }
        let check = Check::new(device);
        let parameters = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("look admission footprint"),
            contents: bytemuck::cast_slice(&values),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let view = image.create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rendered look admission"),
            layout: &self.validation.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: check.status.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: parameters.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.validation);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(image.width().div_ceil(8), image.height().div_ceil(8), 1);
        }
        encoder.copy_buffer_to_buffer(&check.status, 0, &check.readback, 0, 8);
        Ok(check)
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
        let minimum = [look.minimum[0], look.minimum[1], look.minimum[2], 0.];
        let maximum = [look.maximum[0], look.maximum[1], look.maximum[2], 0.];
        let reciprocal = std::array::from_fn(|i| {
            if i < 3 {
                1. / (look.maximum[i] - look.minimum[i])
            } else {
                0.
            }
        });
        let params = Parameters {
            minimum,
            maximum,
            reciprocal,
            dimensions: [count, look.size, x * 64, u32::from(coverage)],
        };
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
