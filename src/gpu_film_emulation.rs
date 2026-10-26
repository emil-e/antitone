//! Shared post-grade AP1 film stage: image MTF, future halation, then grain.
#[cfg(feature = "editor")]
use super::ProcessingParameters;
#[cfg(feature = "editor")]
use crate::frame_raster::FrameRaster;
use crate::{Error, film_emulation::FilmEmulationSettings};
use wgpu::util::DeviceExt;

pub(crate) fn shader() -> String {
    [
        include_str!("shaders/view_geometry.wgsl"),
        include_str!("shaders/film_emulation.wgsl"),
    ]
    .join("\n")
}

pub(crate) struct Kernels {
    field: wgpu::ComputePipeline,
    pub(crate) horizontal: wgpu::ComputePipeline,
    vertical: wgpu::ComputePipeline,
    grain: wgpu::ComputePipeline,
}

pub(crate) struct Workspace {
    pub(crate) scratch: wgpu::Buffer,
    pub(crate) grain: wgpu::Buffer,
}

impl Workspace {
    pub(crate) fn new(device: &wgpu::Device, size: [u32; 2]) -> Result<Self, Error> {
        let bytes = u64::from(size[0]) * u64::from(size[1]) * 16;
        let limits = device.limits();
        if size.contains(&0)
            || bytes > limits.max_storage_buffer_binding_size
            || bytes > limits.max_buffer_size
        {
            return Err(Error::Gpu("film workspace exceeds GPU limits".into()));
        }
        let buffer = |label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        Ok(Self {
            scratch: buffer("bounded film MTF scratch"),
            grain: buffer("bounded source grain"),
        })
    }
}

pub(crate) struct Dispatch<'a> {
    pub(crate) uniform: &'a wgpu::Buffer,
    pub(crate) working: &'a wgpu::Buffer,
    pub(crate) scratch: &'a wgpu::Buffer,
    pub(crate) grain: &'a wgpu::Buffer,
    pub(crate) generate_grain: bool,
    pub(crate) apply_grain: bool,
}

impl Kernels {
    pub(crate) async fn new(device: &wgpu::Device) -> Result<Self, Error> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("source-anchored AP1 film emulation"),
            source: wgpu::ShaderSource::Wgsl(shader().into()),
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
        let kernels = Self {
            field: pipeline("film_field"),
            horizontal: pipeline("film_horizontal"),
            vertical: pipeline("film_vertical"),
            grain: pipeline("film_grain"),
        };
        if let Some(error) = scope.pop().await {
            return Err(Error::Gpu(error.to_string()));
        }
        Ok(kernels)
    }

    pub(crate) fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        dispatch: Dispatch<'_>,
    ) {
        let groups = ((dispatch.working.size() / 16) as u32).div_ceil(64);
        let x = groups.min(device.limits().max_compute_workgroups_per_dimension);
        let mut encode = |pipeline: &wgpu::ComputePipeline,
                          entries: &[wgpu::BindGroupEntry<'_>]| {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("corrected AP1 film stage"),
                layout: &pipeline.get_bind_group_layout(0),
                entries,
            });
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(x, groups.div_ceil(x), 1);
        };
        if dispatch.generate_grain {
            encode(
                &self.field,
                &[
                    wgpu::BindGroupEntry {
                        binding: 10,
                        resource: dispatch.uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 12,
                        resource: dispatch.grain.as_entire_binding(),
                    },
                ],
            );
        }
        for (pipeline, input, output) in [
            (&self.horizontal, dispatch.working, dispatch.scratch),
            (&self.vertical, dispatch.scratch, dispatch.working),
        ] {
            encode(
                pipeline,
                &[
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: output.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 10,
                        resource: dispatch.uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 11,
                        resource: input.as_entire_binding(),
                    },
                ],
            );
        }
        // Future halation consumes the completed image MTF here, before grain is applied.
        if dispatch.apply_grain {
            encode(
                &self.grain,
                &[
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: dispatch.working.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 10,
                        resource: dispatch.uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 12,
                        resource: dispatch.grain.as_entire_binding(),
                    },
                ],
            );
        }
    }
}

#[cfg(feature = "editor")]
fn view_pair(pair: [f64; 2]) -> [f32; 4] {
    [pair[0] as f32, pair[1] as f32, 0., 0.]
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ViewUniform {
    pub(crate) dimensions: [u32; 4],
    pub(crate) origin: [f32; 4],
    pub(crate) dx: [f32; 4],
    pub(crate) dy: [f32; 4],
    pub(crate) gain: [f32; 4],
    pub(crate) source_transform: [glam::Vec4; 3],
    pub(crate) film: [f32; 4],
    pub(crate) film_colour: [f32; 4],
}

impl ViewUniform {
    #[cfg(feature = "editor")]
    pub(crate) fn new(
        frame: FrameRaster,
        parameters: ProcessingParameters,
        film: FilmEmulationSettings,
        seed: u64,
    ) -> Self {
        let origin = frame.source_point([0.5, 0.5]).0;
        let x = frame.source_point([1.5, 0.5]).0;
        let y = frame.source_point([0.5, 1.5]).0;
        // Neutral acquisition omits the response's folded exposure. Restore that factor here;
        // any gain already packed into `post` must not be applied a second time.
        let gain = parameters.anchor.w.exp();
        let (film, film_colour) = match film.parameters(frame.composition.area().height) {
            Ok(Some(parameters)) => {
                let footprint = (x[0] - origin[0])
                    .hypot(x[1] - origin[1])
                    .max((y[0] - origin[0]).hypot(y[1] - origin[1]));
                let variance = parameters.channel_variance(footprint);
                (
                    [
                        1.,
                        parameters.amplitude as f32,
                        parameters.grain_size as f32,
                        0.,
                    ],
                    [
                        parameters.colour as f32,
                        variance[0] as f32,
                        variance[1] as f32,
                        variance[2] as f32,
                    ],
                )
            }
            _ => ([0.; 4], [0.; 4]),
        };
        Self {
            dimensions: [
                frame.output[0],
                frame.output[1],
                seed as u32 ^ ((seed >> 32) as u32).rotate_left(16),
                0,
            ],
            origin: view_pair(origin),
            dx: view_pair(std::array::from_fn(|c| x[c] - origin[c])),
            dy: view_pair(std::array::from_fn(|c| y[c] - origin[c])),
            gain: [gain, 0., 0., 0.],
            source_transform: parameters.post,
            film,
            film_colour,
        }
    }
}

impl ViewUniform {
    pub(crate) fn native(
        bounds: crate::geometry::SourceRegion,
        source_height: u32,
        settings: FilmEmulationSettings,
        seed: u64,
    ) -> Result<Self, Error> {
        use bytemuck::Zeroable;
        let mut uniform = Self {
            dimensions: [
                bounds.width,
                bounds.height,
                seed as u32 ^ ((seed >> 32) as u32).rotate_left(16),
                1,
            ],
            origin: [bounds.x as f32 + 0.5, bounds.y as f32 + 0.5, 0., 0.],
            dx: [1., 0., 0., 0.],
            dy: [0., 1., 0., 0.],
            gain: [1., 0., 0., 0.],
            source_transform: [glam::Vec4::X, glam::Vec4::Y, glam::Vec4::Z],
            ..Self::zeroed()
        };
        if let Some(parameters) = settings.parameters(source_height)? {
            let variance = parameters.channel_variance(1.);
            uniform.film = [
                1.,
                parameters.amplitude as f32,
                parameters.grain_size as f32,
                0.,
            ];
            uniform.film_colour = [
                parameters.colour as f32,
                variance[0] as f32,
                variance[1] as f32,
                variance[2] as f32,
            ];
        }
        Ok(uniform)
    }

    pub(crate) fn buffer(&self, device: &wgpu::Device) -> wgpu::Buffer {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("source film coordinates and parameters"),
            contents: bytemuck::bytes_of(self),
            usage: wgpu::BufferUsages::UNIFORM,
        })
    }
}
