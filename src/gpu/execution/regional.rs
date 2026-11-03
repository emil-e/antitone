//! Original-source compositions with bounded lazy evaluation and affine reconstruction.

use crate::{
    Error,
    gpu::{GpuFlatField, ProcessingInput, SubmissionGate, evaluation},
    renderer::GpuRenderer,
    rendering::{OutputReference, OutputTarget},
    scan::Scan,
};
use std::sync::Arc;

#[cfg(all(test, target_os = "macos"))]
#[path = "regional_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Demand {
    pub area: crate::raw::Rect,
    pub output: [u32; 2],
    pub frame: Option<crate::frame_raster::FrameRaster>,
}

pub(crate) struct Request<'a> {
    pub scan: &'a Scan,
    pub field: Option<Arc<GpuFlatField>>,
    pub input: &'a ProcessingInput,
    pub target: OutputTarget,
    pub demand: Demand,
}

impl Request<'_> {
    pub fn definition(
        self,
        device: &wgpu::Device,
        renderer: &GpuRenderer,
    ) -> Result<Arc<evaluation::ImageNode>, Error> {
        self.validate()?;
        let Self {
            scan,
            field,
            input,
            target,
            demand,
        } = self;
        let root = evaluation::original_chain(
            device,
            renderer,
            scan,
            field,
            input,
            OutputReference::DisplayReferred(target),
            &input.pixel_pipeline(),
        )?;
        let mut grid = if let Some(frame) = demand.frame {
            evaluation::Grid::from_frame(frame)
        } else {
            evaluation::Grid::axis_aligned(
                [f64::from(demand.area.x), f64::from(demand.area.y)],
                [
                    f64::from(demand.area.width) / f64::from(demand.output[0]),
                    f64::from(demand.area.height) / f64::from(demand.output[1]),
                ],
            )
        };
        grid.coverage = 8;
        let domain = evaluation::Domain {
            bounds: Some(crate::gpu::evaluation::CellRegion::new(
                [0, 0],
                demand.output,
            )),
            grid,
            meaning: root.domain().meaning,
        };
        evaluation::reconstruct(root, domain)
    }

    pub fn validate(&self) -> Result<(), Error> {
        self.scan.validate_area(self.demand.area)?;
        if self.scan.flat_field() != self.field.as_ref().map(|field| field.field()) {
            return Err(Error::Gpu("CPU and GPU flat-field resources differ".into()));
        }
        if let Some(frame) = self.demand.frame
            && (frame.composition.area() != self.scan.area
                || frame.bounds()? != self.demand.area
                || frame.output != self.demand.output)
        {
            return Err(Error::InvalidParameter(
                "inconsistent frame refinement request",
            ));
        }
        Ok(())
    }
}

pub(crate) struct Completed {
    pub look_check: Option<Arc<crate::gpu::lut3d::Check>>,
}

pub(crate) fn execute(
    renderer: &GpuRenderer,
    device: &wgpu::Device,
    (queue, gate): (&wgpu::Queue, &SubmissionGate),
    root: Arc<evaluation::ImageNode>,
    check_look: bool,
    output: &wgpu::Texture,
    cancelled: impl Fn() -> bool,
) -> Result<Option<Completed>, Error> {
    if cancelled() {
        return Ok(None);
    }
    let domain = root.domain();
    let grid = domain.grid;
    let bounds = domain.finite()?;
    let size = [bounds.width, bounds.height];
    if domain.meaning != evaluation::Meaning::Display || grid.coverage == 0 {
        return Err(Error::InvalidParameter(
            "regional output requires covered display pixels",
        ));
    }
    if output.width() != size[0]
        || output.height() != size[1]
        || output.format() != wgpu::TextureFormat::Rgba32Float
        || !output.usage().contains(wgpu::TextureUsages::COPY_DST)
    {
        return Err(Error::InvalidParameter(
            "regional output differs from demand",
        ));
    }
    let started = std::time::Instant::now();
    let tile_size = [512, 128];
    let budget = evaluation::Budget {
        tile: tile_size,
        scratch_bytes: (32 << 20).min(device.limits().max_buffer_size),
    };
    let pixels = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("bounded reconstructed reference pixels"),
        size: u64::from(tile_size[0]) * u64::from(tile_size[1]) * 16,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let tile = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bounded reference output transfer"),
        size: wgpu::Extent3d {
            width: tile_size[0],
            height: tile_size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let scope = renderer.evaluation.materializations();
    for y in (0..size[1]).step_by(tile_size[1] as usize) {
        for x in (0..size[0]).step_by(tile_size[0] as usize) {
            if cancelled() {
                return Ok(None);
            }
            let region: crate::geometry::SourceRegion = [
                x,
                y,
                tile_size[0].min(size[0] - x),
                tile_size[1].min(size[1] - y),
            ]
            .into();
            let cells = evaluation::CellRegion::new(
                [
                    i32::try_from(i64::from(bounds.x) + i64::from(x)).map_err(|_| {
                        Error::InvalidParameter("regional output exceeds cell coordinates")
                    })?,
                    i32::try_from(i64::from(bounds.y) + i64::from(y)).map_err(|_| {
                        Error::InvalidParameter("regional output exceeds cell coordinates")
                    })?,
                ],
                [region.width, region.height],
            )
            .validate()?;
            let request = renderer.evaluation.request(
                root.clone(),
                evaluation::Region {
                    bounds: cells,
                    grid,
                },
                budget,
            )?;
            let worker = evaluation::Production {
                context: &renderer.evaluation,
                queue,
                gate,
                scope: &scope,
                budget,
                layout: evaluation::ImageLayout {
                    domain,
                    region: cells,
                    format: evaluation::ImageFormat::Rgba32Float,
                    backing: evaluation::ImageBacking::Buffer,
                },
                pixels: &pixels,
                cancelled: &cancelled,
                progress: &|| {},
            };
            match worker.produce(&request) {
                Ok(_) => {}
                Err(Error::Cancelled) => return Ok(None),
                Err(error) => return Err(error),
            }
            if cancelled() {
                return Ok(None);
            }
            let mut encoder = device.create_command_encoder(&Default::default());
            evaluation::acquire::present_sized(
                &renderer.evaluation,
                &mut encoder,
                &pixels,
                [region.width, region.height],
                &tile,
            )?;
            encoder.copy_texture_to_texture(
                tile.as_image_copy(),
                wgpu::TexelCopyTextureInfo {
                    texture: output,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x, y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: region.width,
                    height: region.height,
                    depth_or_array_layers: 1,
                },
            );
            if renderer
                .evaluation
                .submit(queue, gate, encoder, &cancelled)?
                .is_none()
            {
                return Ok(None);
            }
        }
    }
    if cancelled() {
        return Ok(None);
    }
    let look_check = if check_look {
        let mut encoder = device.create_command_encoder(&Default::default());
        let check = renderer.check_look_image(device, &mut encoder, output);
        if renderer
            .evaluation
            .submit_with(queue, gate, encoder, &cancelled, || check.begin(|| {}))?
            .is_none()
        {
            return Ok(None);
        }
        check
            .result()
            .ok_or_else(|| Error::Gpu("look validation did not complete".into()))??;
        Some(check)
    } else {
        None
    };
    if cancelled() {
        return Ok(None);
    }
    log::debug!(
        output_width=size[0],output_height=size[1],
        seconds=started.elapsed().as_secs_f64();"settled_preview"
    );
    Ok(Some(Completed { look_check }))
}
