use crate::gpu::evaluation::{
    Binding, Domain, Fragment, Grid, Meaning, PlannedImage, Resource, Shared, Storage, Work,
};

#[cfg(test)]
mod look_tests;

pub(crate) use crate::acquisition::windows;

const SHADER: &str = r#"
struct Parameters {
    source: vec2u,
    output: vec2u,
    origin: vec2u,
    tile: vec2u,
    output_origin: vec2u,
    extent: vec2u,
}

@group(0) @binding(0)
var image: texture_2d<f32>;

@group(0) @binding(1)
var<storage, read_write> sums: array<vec4f>;

@group(0) @binding(2)
var<uniform> p: Parameters;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if any(id.xy >= p.extent) {
        return;
    }

    let xy = id.xy + p.output_origin;
    let scale = vec2f(p.source) / vec2f(p.output);
    let start = vec2f(xy) * scale;
    let end = vec2f(xy + vec2u(1)) * scale;
    let lo = max(vec2u(floor(start)), p.origin);
    let hi = min(min(vec2u(ceil(end)), p.origin + p.tile), p.source);
    if any(lo >= hi) {
        return;
    }

    var sum = vec3f(0.);
    var invalid = 0.;
    for(var y = lo.y; y < hi.y; y++) {
        let wy = max(0., min(end.y, f32(y + 1)) - max(start.y, f32(y)));
        for(var x = lo.x; x < hi.x; x++) {
            let wx = max(0., min(end.x, f32(x + 1)) - max(start.x, f32(x)));
            let pixel = textureLoad(image, vec2i(vec2u(x, y) - p.origin), 0);
            if pixel.a < 0. && wx * wy > 0. {
                invalid = max(invalid, -pixel.a);
            }
            sum += pixel.rgb * wx * wy;
        }
    }

    let index = xy.x + xy.y * p.output.x;
    sums[index] = vec4f(sums[index].rgb + sum / (scale.x * scale.y), max(sums[index].a, invalid));
}
"#;

const FINISH: &str = r#"
@group(0) @binding(0)
var<storage, read> sums: array<vec4f>;

@group(0) @binding(1)
var<uniform> size: vec4u;

@vertex
fn vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let p = array<vec2f, 3>(vec2f(-1., -1.), vec2f(3., -1.), vec2f(-1., 3.))[i];
    return vec4f(p, 0., 1.);
}

@fragment
fn fragment(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let xy = vec2u(position.xy);
    let value = sums[xy.x + xy.y * size.x];
    return vec4f(value.rgb, select(1., -value.a, value.a > 0.));
}
"#;

pub(crate) struct AveragePipelines {
    evaluation: crate::gpu::evaluation::Context,
    compute: [std::sync::Arc<str>; 2],
    finish: wgpu::RenderPipeline,
}

impl AveragePipelines {
    pub(crate) async fn new(device: &wgpu::Device) -> Result<Self, crate::Error> {
        Self::with_context(
            device,
            crate::gpu::evaluation::Context::new(device, Default::default()),
        )
        .await
    }

    pub(crate) async fn with_context(
        device: &wgpu::Device,
        evaluation: crate::gpu::evaluation::Context,
    ) -> Result<Self, crate::Error> {
        evaluation.require_device(device)?;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let affine = [
            include_str!("shaders/footprint.wgsl"),
            include_str!("shaders/affine_area.wgsl"),
        ]
        .join("\n");
        let compute: [std::sync::Arc<str>; 2] = [SHADER.into(), affine.into()];
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
        evaluation.prewarm(
            &compute
                .iter()
                .map(|code| Work::Compute {
                    code: code.clone(),
                    entry: "main",
                    bindings: vec![
                        (0, Binding::Texture(Resource(0))),
                        (1, Binding::Write(Resource(1))),
                        (2, Binding::Uniform(Resource(2))),
                    ],
                    groups: [1, 1, 1],
                    initialize: None,
                })
                .collect::<Vec<_>>(),
        )?;
        Ok(Self {
            evaluation,
            compute,
            finish,
        })
    }
}

pub struct AreaAverage {
    pipelines: std::sync::Arc<AveragePipelines>,
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
        let pipelines = std::sync::Arc::new(AveragePipelines::new(device).await?);
        Self::new_shared(device, pipelines, source, output)
    }

    pub(crate) fn new_shared(
        device: &wgpu::Device,
        pipelines: std::sync::Arc<AveragePipelines>,
        source: [u32; 2],
        output: [u32; 2],
    ) -> Result<Self, crate::Error> {
        Self::create(device, pipelines, source, output, None)
    }

    pub async fn new_affine(
        device: &wgpu::Device,
        raster: crate::frame_raster::FrameRaster,
    ) -> Result<Self, crate::Error> {
        let pipelines = std::sync::Arc::new(AveragePipelines::new(device).await?);
        Self::new_affine_shared(device, pipelines, raster)
    }

    pub(crate) fn new_affine_shared(
        device: &wgpu::Device,
        pipelines: std::sync::Arc<AveragePipelines>,
        raster: crate::frame_raster::FrameRaster,
    ) -> Result<Self, crate::Error> {
        let area = raster.bounds()?;
        Self::create(
            device,
            pipelines,
            [area.width, area.height],
            raster.output,
            Some(raster.affine()?),
        )
    }

    fn create(
        device: &wgpu::Device,
        pipelines: std::sync::Arc<AveragePipelines>,
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
            || affine.is_some_and(|a| {
                let det = a[1][0] * a[2][1] - a[1][1] * a[2][0];
                !det.is_finite() || det == 0.
            })
        {
            return Err(crate::Error::Gpu(
                "invalid area-average dimensions or GPU budget".into(),
            ));
        }
        let sums = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("linear target RGB area sums"),
            size,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        Ok(Self {
            pipelines,
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
    ) -> Result<(), crate::Error> {
        for window in self.windows(tile, origin) {
            self.accumulate_window(device, encoder, tile, origin, window)?;
        }
        Ok(())
    }

    pub(crate) fn windows(&self, tile: &wgpu::Texture, origin: [u32; 2]) -> Vec<windows::Window> {
        let output = [self.dimensions[2], self.dimensions[3]];
        windows::bounded(if let Some(affine) = self.affine {
            windows::affine_output(affine, output, origin, [tile.width(), tile.height()])
        } else {
            overlapping_output(
                [self.dimensions[0], self.dimensions[1]],
                output,
                origin,
                [tile.width(), tile.height()],
            )
        })
    }

    pub(crate) fn accumulate_window(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        tile: &wgpu::Texture,
        origin: [u32; 2],
        (output_origin, extent): windows::Window,
    ) -> Result<(), crate::Error> {
        self.pipelines.evaluation.require_device(device)?;
        if extent.contains(&0) {
            return Ok(());
        }
        let size = [self.dimensions[2], self.dimensions[3]];
        if (0..2).any(|i| {
            output_origin[i]
                .checked_add(extent[i])
                .is_none_or(|end| end > size[i])
        }) || !matches!(
            tile.format().sample_type(None, None),
            Some(wgpu::TextureSampleType::Float { .. })
        ) || !tile.usage().contains(wgpu::TextureUsages::TEXTURE_BINDING)
        {
            return Err(crate::Error::InvalidParameter(
                "invalid reconstruction window or source texture",
            ));
        }
        let parameters = [
            self.dimensions[0],
            self.dimensions[1],
            self.dimensions[2],
            self.dimensions[3],
            origin[0],
            origin[1],
            tile.width(),
            tile.height(),
            output_origin[0],
            output_origin[1],
            extent[0],
            extent[1],
        ];
        let mut bytes = if self.affine.is_some() {
            bytemuck::cast_slice(&parameters[..8]).to_vec()
        } else {
            bytemuck::cast_slice(&parameters).to_vec()
        };
        if let Some(affine) = self.affine {
            bytes.extend_from_slice(bytemuck::cast_slice(&affine));
            bytes.extend_from_slice(bytemuck::cast_slice(&[0_f32; 2]));
            bytes.extend_from_slice(bytemuck::cast_slice(&parameters[8..]));
        }
        let mut fragment = Fragment::default();
        let output = Self::image(
            &mut fragment,
            [self.dimensions[2], self.dimensions[3]],
            Storage::Input {
                slot: 0,
                bytes: self.sums.size(),
                writable: true,
            },
        );
        let uniform = fragment.resource(Storage::Uniform(bytes.into()));
        fragment.work.push(Work::Compute {
            code: self.pipelines.compute[usize::from(self.affine.is_some())].clone(),
            entry: "main",
            bindings: vec![
                (
                    0,
                    Binding::Shared(Shared::Texture(tile.create_view(&Default::default()))),
                ),
                (1, Binding::Write(output.resource)),
                (2, Binding::Uniform(uniform)),
            ],
            groups: [extent[0].div_ceil(8), extent[1].div_ceil(8), 1],
            initialize: None,
        });
        self.pipelines
            .evaluation
            .encode_work(fragment, output, encoder, &[&self.sums])
    }

    pub fn finish(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        output: &wgpu::Texture,
    ) -> Result<(), crate::Error> {
        self.pipelines.evaluation.require_device(device)?;
        let size = [self.dimensions[2], self.dimensions[3]];
        if [output.width(), output.height()] != size {
            return Err(crate::Error::InvalidParameter(
                "reconstruction attachment differs from output grid",
            ));
        }
        let mut fragment = Fragment::default();
        let sums = fragment.resource(Storage::Input {
            slot: 0,
            bytes: self.sums.size(),
            writable: false,
        });
        let uniform = fragment.uniform(&[size[0], size[1], 0u32, 0]);
        let image = Self::image(&mut fragment, size, Storage::RenderTarget(output.clone()));
        fragment.work.push(Work::Render {
            pipeline: self.pipelines.finish.clone(),
            layout: self.pipelines.finish.get_bind_group_layout(0),
            bindings: vec![(0, Binding::Read(sums)), (1, uniform)],
            target: image.resource,
        });
        self.pipelines
            .evaluation
            .encode_work(fragment, image, encoder, &[&self.sums])
    }

    fn image(fragment: &mut Fragment, size: [u32; 2], storage: Storage) -> PlannedImage {
        PlannedImage {
            resource: fragment.resource(storage),
            region: [0, 0, size[0], size[1]].into(),
            domain: Domain {
                bounds: [0, 0, size[0], size[1]].into(),
                grid: Grid::axis_aligned([0.; 2], [1.; 2]),
                meaning: Meaning::Display,
            },
        }
    }
}

fn overlapping_output(
    source: [u32; 2],
    output: [u32; 2],
    origin: [u32; 2],
    tile: [u32; 2],
) -> ([u32; 2], [u32; 2]) {
    let mut start = [0; 2];
    let mut extent = [0; 2];
    for axis in 0..2 {
        if origin[axis] >= source[axis] || tile[axis] == 0 {
            return (start, [0; 2]);
        }
        let denominator = u64::from(source[axis]);
        let left = u64::from(origin[axis]) * u64::from(output[axis]);
        let right = u64::from(origin[axis].saturating_add(tile[axis]).min(source[axis]))
            * u64::from(output[axis]);
        // Include one boundary pixel for the shader's f32 footprint arithmetic.
        start[axis] = (left / denominator).saturating_sub(1) as u32;
        let end = (right.div_ceil(denominator) + 1).min(u64::from(output[axis])) as u32;
        extent[axis] = end - start[axis];
    }
    (start, extent)
}

#[cfg(test)]
mod tests {
    use super::overlapping_output;

    #[test]
    fn dispatch_bounds_cover_fractional_footprints_and_ignore_padding() {
        for source in [[15, 9], [8640, 5760], [31, 1]] {
            for output in [[1, 1], [4.min(source[0]), 3.min(source[1])], source] {
                for y in (0..source[1]).step_by(7) {
                    for x in (0..source[0]).step_by(11) {
                        let (start, extent) = overlapping_output(source, output, [x, y], [11, 7]);
                        for axis in 0..2 {
                            let origin = [x, y][axis] as f64;
                            let end = (origin + [11., 7.][axis]).min(source[axis] as f64);
                            let scale = source[axis] as f64 / output[axis] as f64;
                            assert!(start[axis] as f64 * scale <= origin);
                            assert!((start[axis] + extent[axis]) as f64 * scale >= end);
                            assert!(start[axis] + extent[axis] <= output[axis]);
                        }
                    }
                }
            }
        }
        assert_eq!(
            overlapping_output([15, 9], [4, 3], [15, 0], [11, 7]).1,
            [0, 0]
        );
    }

    #[test]
    fn full_frame_tiles_dispatch_only_a_small_fraction_of_the_old_work() {
        let mut invocations = 0u64;
        for y in (0..5760).step_by(256) {
            for x in (0..8640).step_by(1024) {
                let (_, extent) =
                    overlapping_output([8640, 5760], [5120, 3414], [x, y], [1024, 256]);
                invocations +=
                    u64::from(extent[0].div_ceil(8)) * u64::from(extent[1].div_ceil(8)) * 64;
            }
        }
        assert!(invocations < 20_000_000, "{invocations}");
    }
}
