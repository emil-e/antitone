//! Immutable 3D-LUT resources and rendered-output admission.
use crate::{
    Error,
    looks::{LookId, Lut3D},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use wgpu::util::DeviceExt;

#[cfg(test)]
mod test_dispatch;
#[cfg(test)]
pub(crate) use test_dispatch::Kernel;
#[cfg(all(test, target_os = "macos"))]
mod tests;

fn calculation(
    id: LookId,
    parameters: Parameters,
    nodes: crate::gpu::evaluation::FunctionBinding,
) -> crate::gpu::evaluation::value::Operation {
    use crate::gpu::evaluation::{Function, value};
    let code = [
        include_str!("lut3d/arguments.wgsl"),
        include_str!("lut3d/value.wgsl"),
        "fn pixel(cell:vec2i,position:vec2f,value:vec4f)->vec4f {return look_pixel(value);}",
    ]
    .join("\n");
    let captures = Function::new(code.clone(), "pixel").with_uniform(3, &parameters);
    let mut bindings = captures.bindings;
    bindings.push((2, nodes));
    value::Operation::new(
        "ACEScct/AP1 tetrahedral look v1",
        id,
        code,
        "pixel",
        value::Access::Pointwise,
    )
    .with_bindings(bindings)
}

pub(crate) fn prewarm_calculation(coverage: bool) -> crate::gpu::evaluation::value::Operation {
    let parameters = Parameters {
        minimum: [0.; 4],
        maximum: [1.; 4],
        reciprocal: [1.; 4],
        dimensions: [0, 2, 0, u32::from(coverage)],
    };
    calculation(
        LookId([0; 32]),
        parameters,
        crate::gpu::evaluation::FunctionBinding::Storage(
            bytemuck::cast_slice(&[[0f32; 4]; 8]).into(),
        ),
    )
}

pub(crate) fn prewarm(context: &crate::gpu::evaluation::Context) -> Result<(), Error> {
    use crate::gpu::evaluation::*;
    let domain = Domain {
        bounds: Some(CellRegion::new([0, 0], [1, 1])),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Graded,
    };
    context.prewarm_image(value::pointwise(
        borrowed_input(domain),
        Meaning::Graded,
        prewarm_calculation(false),
    )?)
}

#[cfg(test)]
pub(crate) fn shader() -> String {
    [
        include_str!("lut3d/arguments.wgsl"),
        include_str!("lut3d/value.wgsl"),
        include_str!("lut3d.wgsl"),
    ]
    .join("\n")
}

pub(crate) struct Validation {
    pipeline: wgpu::ComputePipeline,
}
impl Validation {
    pub async fn new(device: &wgpu::Device) -> Result<Self, Error> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let validation_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rendered look admission"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}",
                    include_str!("evaluation/footprint.wgsl"),
                    include_str!("lut3d_validation.wgsl")
                )
                .into(),
            ),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
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
        Ok(Self { pipeline })
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
            layout: &self.pipeline.get_bind_group_layout(0),
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
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(image.width().div_ceil(8), image.height().div_ceil(8), 1);
        }
        encoder.copy_buffer_to_buffer(&check.status, 0, &check.readback, 0, 8);
        Ok(check)
    }
}

pub struct Lut {
    pub(super) buffer: wgpu::Buffer,
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

    pub(crate) fn calculation(&self, coverage: bool) -> crate::gpu::evaluation::value::Operation {
        calculation(
            self.id,
            self.parameters(0, 0, coverage),
            crate::gpu::evaluation::FunctionBinding::Shared(
                crate::gpu::evaluation::Shared::Buffer(self.buffer.clone()),
            ),
        )
    }

    pub(super) fn parameters(&self, count: u32, stride: u32, coverage: bool) -> Parameters {
        Parameters {
            minimum: [self.minimum[0], self.minimum[1], self.minimum[2], 0.],
            maximum: [self.maximum[0], self.maximum[1], self.maximum[2], 0.],
            reciprocal: std::array::from_fn(|i| {
                if i < 3 {
                    1. / (self.maximum[i] - self.minimum[i])
                } else {
                    0.
                }
            }),
            dimensions: [count, self.size, stride, u32::from(coverage)],
        }
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
pub(super) struct Parameters {
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
    pub(crate) fn new(device: &wgpu::Device) -> Arc<Self> {
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
