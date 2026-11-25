//! Original-source compositions with bounded lazy evaluation and affine reconstruction.

use crate::Error;
use crate::evaluation;

#[cfg(all(test, target_os = "macos"))]
#[path = "regional_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Demand {
    pub area: crate::raw::Rect,
    pub output: [u32; 2],
    pub frame: Option<crate::frame_raster::FrameRaster>,
}

fn validate_demand(area: crate::raw::Rect, demand: Demand) -> Result<(), Error> {
    if let Some(frame) = demand.frame
        && (frame.composition.area() != area
            || frame.bounds()? != demand.area
            || frame.output != demand.output)
    {
        return Err(Error::InvalidParameter(
            "inconsistent frame refinement request",
        ));
    }
    Ok(())
}

pub(crate) fn definition(
    root: evaluation::Value,
    demand: Demand,
) -> Result<evaluation::Value, Error> {
    let area = root.domain().finite()?.source()?;
    if !root
        .domain()
        .finite()?
        .contains_region(demand.area.try_into()?)
    {
        return Err(Error::InvalidParameter(
            "refinement demand exceeds the source",
        ));
    }
    validate_demand(area, demand)?;
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
        bounds: Some(crate::evaluation::CellRegion::new([0, 0], demand.output)),
        grid,
    };
    crate::reconstruction::reconstruct(root, domain)
}

pub(crate) fn execute(
    context: &evaluation::Context,
    device: &wgpu::Device,
    queue: &crate::gpu::submission::Queue,
    root: evaluation::Value,
    output: &wgpu::Texture,
    cancelled: impl Fn() -> bool,
) -> Result<Option<()>, Error> {
    if cancelled() {
        return Ok(None);
    }
    let domain = root.domain();
    let grid = domain.grid;
    let bounds = domain.finite()?;
    let size = [bounds.width, bounds.height];
    if grid.coverage == 0 {
        return Err(Error::InvalidParameter(
            "regional output requires covered pixels",
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
    let budget = evaluation::Tiling { tile: tile_size };
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
    let scope = context.materializations();
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
            let request = context.request(
                root.clone(),
                evaluation::Region {
                    bounds: cells,
                    grid,
                },
                budget,
            )?;
            let worker = evaluation::Production {
                worker: evaluation::Worker {
                    context,
                    queue,
                    scope: &scope,
                    working: &evaluation::RunWorkingSet::default(),
                    cancelled: &cancelled,
                    progress: &|| {},
                },
                layout: evaluation::ImageLayout {
                    domain,
                    region: cells,
                    format: evaluation::ImageFormat::Rgba32Float,
                    backing: evaluation::ImageBacking::Buffer,
                },
                pixels: &pixels,
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
                context,
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
            if context.submit(queue, encoder, &cancelled)?.is_none() {
                return Ok(None);
            }
        }
    }
    if cancelled() {
        return Ok(None);
    }
    log::debug!(
        output_width=size[0],output_height=size[1],
        seconds=started.elapsed().as_secs_f64();"settled_preview"
    );
    Ok(Some(()))
}
