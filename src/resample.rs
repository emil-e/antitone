//! Incremental supplied-texture adapter for the shared image reconstruction calculation.
use crate::gpu::evaluation::{
    self, Binding, Domain, Fragment, Grid, Meaning, PlannedImage, Shared, Storage, Work,
};
use std::sync::Arc;

#[cfg(test)]
mod look_tests;

pub(crate) use crate::acquisition::windows;

const FINISH: &str = r#"
@group(0) @binding(0) var<storage,read> means:array<vec4f>;
@group(0) @binding(1) var<storage,read> flags:array<f32>;
@group(0) @binding(2) var<uniform> parameters:vec4u;

@vertex
fn vertex(@builtin(vertex_index) i:u32)->@builtin(position) vec4f {
    let p=array<vec2f,3>(vec2f(-1.,-1.),vec2f(3.,-1.),vec2f(-1.,3.))[i];
    return vec4f(p,0.,1.);
}
@fragment
fn fragment(@builtin(position) position:vec4f)->@location(0) vec4f {
    let xy=vec2u(position.xy);
    let index=xy.x+xy.y*parameters.x;
    let value=means[index];
    let status=reconstruction_status(flags[index],value.w,0u,0.0);
    // This incremental adapter leaves as-yet-unsupplied source area black.
    let fraction=value.w/bitcast<f32>(parameters.z);
    return vec4f(value.rgb*fraction,select(1.0,status,status<0.0));
}
"#;

pub(crate) struct AveragePipelines {
    evaluation: evaluation::Context,
    finish: wgpu::RenderPipeline,
}
impl AveragePipelines {
    pub(crate) async fn new(device: &wgpu::Device) -> Result<Self, crate::Error> {
        Self::with_context(device, evaluation::Context::new(device, Default::default())).await
    }
    pub(crate) async fn with_context(
        device: &wgpu::Device,
        evaluation: evaluation::Context,
    ) -> Result<Self, crate::Error> {
        evaluation.require_device(device)?;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("reconstruction texture delivery"),
            source: wgpu::ShaderSource::Wgsl(
                [
                    include_str!("gpu/evaluation/reconstruction_status.wgsl"),
                    FINISH,
                ]
                .join("\n")
                .into(),
            ),
        });
        let finish = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("reconstruction texture delivery"),
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
        Ok(Self { evaluation, finish })
    }
}

pub struct AreaAverage {
    pipelines: Arc<AveragePipelines>,
    means: wgpu::Buffer,
    flags: wgpu::Buffer,
    dimensions: [u32; 4],
    affine: Option<[[f32; 2]; 3]>,
    image: evaluation::Value,
    expected: f32,
}
impl AreaAverage {
    pub async fn new(
        device: &wgpu::Device,
        source: [u32; 2],
        output: [u32; 2],
    ) -> Result<Self, crate::Error> {
        let pipelines = Arc::new(AveragePipelines::new(device).await?);
        Self::new_shared(device, pipelines, source, output)
    }
    pub(crate) fn new_shared(
        device: &wgpu::Device,
        pipelines: Arc<AveragePipelines>,
        source: [u32; 2],
        output: [u32; 2],
    ) -> Result<Self, crate::Error> {
        Self::create(device, pipelines, source, output, None)
    }
    pub async fn new_affine(
        device: &wgpu::Device,
        raster: crate::frame_raster::FrameRaster,
    ) -> Result<Self, crate::Error> {
        let pipelines = Arc::new(AveragePipelines::new(device).await?);
        Self::new_affine_shared(device, pipelines, raster)
    }
    pub(crate) fn new_affine_shared(
        device: &wgpu::Device,
        pipelines: Arc<AveragePipelines>,
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
        pipelines: Arc<AveragePipelines>,
        source: [u32; 2],
        output: [u32; 2],
        affine: Option<[[f32; 2]; 3]>,
    ) -> Result<Self, crate::Error> {
        let bytes = (u64::from(output[0]) * u64::from(output[1]))
            .checked_mul(16)
            .ok_or_else(|| crate::Error::Gpu("area-average size overflow".into()))?;
        if source.contains(&0)
            || output.contains(&0)
            || (affine.is_none() && (output[0] > source[0] || output[1] > source[1]))
            || bytes > device.limits().max_storage_buffer_binding_size
            || bytes > device.limits().max_buffer_size
        {
            return Err(crate::Error::Gpu(
                "invalid area-average dimensions or GPU budget".into(),
            ));
        }
        let mapping = affine.unwrap_or([
            [0.; 2],
            [source[0] as f32 / output[0] as f32, 0.],
            [0., source[1] as f32 / output[1] as f32],
        ]);
        let expected = (mapping[1][0] * mapping[2][1] - mapping[1][1] * mapping[2][0]).abs();
        if !expected.is_finite() || expected == 0. {
            return Err(crate::Error::Gpu(
                "invalid area-average dimensions or GPU budget".into(),
            ));
        }
        let input = Domain {
            bounds: Some(crate::gpu::evaluation::CellRegion::new([0, 0], source)),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::Display,
        };
        let domain = Domain {
            bounds: Some(crate::gpu::evaluation::CellRegion::new([0, 0], output)),
            grid: Grid {
                origin: mapping[0].map(f64::from),
                axes: [mapping[1].map(f64::from), mapping[2].map(f64::from)],
                pitch: [mapping[1], mapping[2]].map(|a| f64::from(a[0]).hypot(f64::from(a[1]))),
                coverage: 8,
            },
            meaning: Meaning::Display,
        };
        let image = evaluation::reconstruct_complete(evaluation::borrowed_input(input), domain)?;
        let buffer = |size, label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        };
        Ok(Self {
            pipelines,
            means: buffer(bytes, "required reconstruction means"),
            flags: buffer(bytes / 4, "required reconstruction flags"),
            dimensions: [source[0], source[1], output[0], output[1]],
            affine,
            image,
            expected,
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
        let samples = crate::geometry::SourceRegion::from([
            origin[0],
            origin[1],
            tile.width(),
            tile.height(),
        ])
        .validate()?;
        let affected = [output_origin[0], output_origin[1], extent[0], extent[1]].try_into()?;
        let mut fragment = Fragment::default();
        let output = self.means(&mut fragment);
        let metadata = fragment.resource(Storage::Input {
            slot: 1,
            bytes: self.flags.size(),
            writable: true,
        });
        let rectangle = fragment.uniform(&[origin[0], origin[1], tile.width(), tile.height()]);
        let sample=evaluation::Sampled {code:r#"
@group(0) @binding(32) var supplied_image:texture_2d<f32>;
@group(0) @binding(33) var<uniform> supplied_rectangle:vec4u;
fn supplied_sample(p:vec2i)->vec4f {return textureLoad(supplied_image,p-vec2i(supplied_rectangle.xy),0);}
"#.into(),symbol:"supplied_sample".into(),bindings:vec![
            (32,Binding::Shared(Shared::Texture(tile.create_view(&Default::default())))),(33,rectangle),
        ]};
        evaluation::accumulate_supplied(
            &self.image,
            &mut fragment,
            sample,
            (samples.try_into()?, affected),
            output,
            metadata,
        )?;
        self.pipelines.evaluation.encode_work(
            fragment,
            output,
            encoder,
            &[&self.means, &self.flags],
        )
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
        let means = fragment.resource(Storage::Input {
            slot: 0,
            bytes: self.means.size(),
            writable: false,
        });
        let flags = fragment.resource(Storage::Input {
            slot: 1,
            bytes: self.flags.size(),
            writable: false,
        });
        let parameters = fragment.uniform(&[size[0], size[1], self.expected.to_bits(), 0]);
        let image = PlannedImage {
            resource: fragment.resource(Storage::RenderTarget(output.clone())),
            domain: self.image.domain(),
            region: self
                .image
                .domain()
                .finite()
                .expect("area averaging has a fixed output"),
        };
        fragment.work.push(Work::Render {
            program: evaluation::RenderProgram::Captured {
                pipeline: self.pipelines.finish.clone(),
                layout: self.pipelines.finish.get_bind_group_layout(0),
            },
            bindings: vec![
                (0, Binding::Read(means)),
                (1, Binding::Read(flags)),
                (2, parameters),
            ],
            target: image.resource,
        });
        self.pipelines
            .evaluation
            .encode_work(fragment, image, encoder, &[&self.means, &self.flags])
    }
    fn means(&self, fragment: &mut Fragment) -> PlannedImage {
        PlannedImage {
            resource: fragment.resource(Storage::Input {
                slot: 0,
                bytes: self.means.size(),
                writable: true,
            }),
            domain: self.image.domain(),
            region: self
                .image
                .domain()
                .finite()
                .expect("area averaging has a fixed output"),
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
