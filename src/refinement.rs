use crate::{
    Error,
    gpu::{GpuFlatField, ProcessingInput, SourceGeometry},
    rendering::OutputTarget,
    resample::AreaAverage,
    scan::Scan,
    viewer::GpuImage,
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewRequest {
    pub area: crate::raw::Rect,
    pub output: [u32; 2],
    pub frame: Option<crate::frame_raster::FrameRaster>,
}

#[allow(clippy::too_many_arguments)]
pub fn refine(
    renderer: &Arc<crate::renderer::GpuRenderer>,
    device: &wgpu::Device,
    (queue, submissions): (&wgpu::Queue, &crate::platform::SubmissionGate),
    (scan, field): (&Scan, Option<Arc<GpuFlatField>>),
    parameters: impl Into<ProcessingInput>,
    target: OutputTarget,
    (generation, expected): (&AtomicU64, u64),
    view: ViewRequest,
) -> Result<Option<Arc<GpuImage>>, Error> {
    let input = parameters.into();
    let start = std::time::Instant::now();
    if generation.load(Ordering::Relaxed) != expected {
        return Ok(None);
    }
    scan.validate_area(view.area)?;
    if scan.flat_field() != field.as_ref().map(|field| field.field()) {
        return Err(Error::Gpu("CPU and GPU flat-field resources differ".into()));
    }
    let [width, height] = view.output;
    let output = pollster::block_on(GpuImage::new_rendered_shared(
        renderer, device, width, height, target,
    ))?;
    let tile_width = 512;
    let tile_height = 128;
    let film = input.film_emulation().parameters(scan.area.height)?;
    let mut expanded: Option<GpuImage> = None;
    let mut tile = if film.is_some() {
        pollster::block_on(GpuImage::new_rendered_shared(
            renderer,
            device,
            tile_width,
            tile_height,
            target,
        ))?
    } else {
        let zeros = vec![[0.; 4]; tile_width as usize * tile_height as usize];
        let mut tile = pollster::block_on(GpuImage::new_continuous_shared(
            renderer,
            device,
            tile_width,
            tile_height,
            &zeros,
            input
                .grade()
                .map_or(scan.camera_to_ap1, |grade| grade.source_matrix()),
            target,
        ))?;
        tile.set_flat_field(
            field.clone(),
            SourceGeometry::new(
                [view.area.x, view.area.y, tile_width, tile_height],
                [tile_width, tile_height],
            )?,
        )?;
        tile.require_processing(&input)?;
        tile
    };
    let average = if let Some(frame) = view.frame {
        if frame.bounds()? != view.area || frame.output != view.output {
            return Err(Error::InvalidParameter(
                "inconsistent frame refinement request",
            ));
        }
        AreaAverage::new_affine_shared(device, renderer.average.clone(), frame)?
    } else {
        AreaAverage::new_shared(
            device,
            renderer.average.clone(),
            [view.area.width, view.area.height],
            [width, height],
        )?
    };
    let mut sampling = std::time::Duration::ZERO;
    let mut gpu = std::time::Duration::ZERO;
    for y in (0..view.area.height).step_by(tile_height as usize) {
        for x in (0..view.area.width).step_by(tile_width as usize) {
            if generation.load(Ordering::Relaxed) != expected {
                return Ok(None);
            }
            let tick = std::time::Instant::now();
            let central = crate::geometry::SourceRegion::from([
                view.area.x + x,
                view.area.y + y,
                tile_width.min(view.area.width - x),
                tile_height.min(view.area.height - y),
            ]);
            let film_bounds = if let Some(parameters) = film {
                let radius = parameters.support_radius();
                let left = central.x.saturating_sub(radius).max(scan.area.x);
                let top = central.y.saturating_sub(radius).max(scan.area.y);
                let right = (central.x + central.width)
                    .saturating_add(radius)
                    .min(scan.area.x + scan.area.width);
                let bottom = (central.y + central.height)
                    .saturating_add(radius)
                    .min(scan.area.y + scan.area.height);
                Some(crate::geometry::SourceRegion::from([
                    left,
                    top,
                    right - left,
                    bottom - top,
                ]))
            } else {
                None
            };
            if let Some(bounds) = film_bounds {
                let size = [bounds.width, bounds.height];
                let bytes = u64::from(size[0]) * u64::from(size[1]) * 16;
                if size
                    .into_iter()
                    .any(|n| n > device.limits().max_texture_dimension_2d)
                    || bytes > device.limits().max_storage_buffer_binding_size
                {
                    return Err(Error::Gpu(
                        "film refinement support exceeds GPU limits".into(),
                    ));
                }
                let pixels = scan.tile_region(
                    scan.area,
                    bounds.x - scan.area.x,
                    bounds.y - scan.area.y,
                    size[0],
                    size[1],
                )?;
                if expanded.as_ref().is_none_or(|image| {
                    image.linear.width() != size[0] || image.linear.height() != size[1]
                }) {
                    expanded = Some(pollster::block_on(GpuImage::new_continuous_shared(
                        renderer,
                        device,
                        size[0],
                        size[1],
                        &pixels,
                        scan.camera_to_ap1,
                        target,
                    ))?);
                } else {
                    expanded
                        .as_ref()
                        .unwrap()
                        .write_source_continuous(queue, &pixels)?;
                }
                expanded.as_mut().unwrap().set_flat_field(
                    field.clone(),
                    SourceGeometry::original_tile(
                        [bounds.x, bounds.y, bounds.width, bounds.height],
                        size,
                    )?,
                )?;
            } else {
                let pixels = scan.tile_region(view.area, x, y, tile_width, tile_height)?;
                tile.write_source_continuous(queue, &pixels)?;
                tile.set_geometry(SourceGeometry::new(
                    [view.area.x + x, view.area.y + y, tile_width, tile_height],
                    [tile_width, tile_height],
                )?)?;
            }
            sampling += tick.elapsed();
            let tick = std::time::Instant::now();
            for (index, window) in average
                .windows(&tile.linear, [x, y])
                .into_iter()
                .enumerate()
            {
                if generation.load(Ordering::Relaxed) != expected {
                    return Ok(None);
                }
                let mut encoder = device.create_command_encoder(&Default::default());
                if index == 0 {
                    if let Some(bounds) = film_bounds {
                        let source = expanded.as_ref().unwrap();
                        source.encode_processing(device, &mut encoder, &input)?;
                        encoder.copy_texture_to_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture: &source.linear,
                                mip_level: 0,
                                origin: wgpu::Origin3d {
                                    x: central.x - bounds.x,
                                    y: central.y - bounds.y,
                                    z: 0,
                                },
                                aspect: wgpu::TextureAspect::All,
                            },
                            tile.linear.as_image_copy(),
                            wgpu::Extent3d {
                                width: central.width,
                                height: central.height,
                                depth_or_array_layers: 1,
                            },
                        );
                    } else {
                        tile.encode_processing(device, &mut encoder, &input)?;
                    }
                }
                average.accumulate_window(device, &mut encoder, &tile.linear, [x, y], window);
                wait_for(device, submissions.submit(queue, [encoder.finish()]))?;
            }
            gpu += tick.elapsed();
        }
    }
    if generation.load(Ordering::Relaxed) != expected {
        return Ok(None);
    }
    let mut encoder = device.create_command_encoder(&Default::default());
    average.finish(device, &mut encoder, &output.linear);
    if input.look().is_some() {
        output.set_look_check(Some(renderer.check_look_image(
            device,
            &mut encoder,
            &output.linear,
        )));
    }
    let submission = submissions.submit(queue, [encoder.finish()]);
    output.begin_look_check(|| {});
    wait_for(device, submission)?;
    output
        .look_result()
        .ok_or_else(|| Error::Gpu("look validation did not complete".into()))??;
    if generation.load(Ordering::Relaxed) != expected {
        return Ok(None);
    }
    log::debug!(
        source_width = view.area.width,
        source_height = view.area.height,
        output_width = width,
        output_height = height,
        target:?,
        seconds = start.elapsed().as_secs_f64(),
        sampling_seconds = sampling.as_secs_f64(),
        gpu_seconds = gpu.as_secs_f64();
        "settled_preview"
    );
    Ok(Some(Arc::new(output)))
}

fn wait_for(device: &wgpu::Device, submission: wgpu::SubmissionIndex) -> Result<(), Error> {
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .map_err(|e| Error::Gpu(e.to_string()))?;
    Ok(())
}
