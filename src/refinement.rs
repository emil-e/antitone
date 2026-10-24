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
    let zeros = vec![[0.; 4]; tile_width as usize * tile_height as usize];
    let mut tile = pollster::block_on(GpuImage::new_continuous_shared(
        renderer,
        device,
        tile_width,
        tile_height,
        &zeros,
        scan.camera_to_ap1,
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
            let pixels = scan.tile_region(view.area, x, y, tile_width, tile_height)?;
            sampling += tick.elapsed();
            let tick = std::time::Instant::now();
            tile.write_source_continuous(queue, &pixels)?;
            tile.set_geometry(SourceGeometry::new(
                [view.area.x + x, view.area.y + y, tile_width, tile_height],
                [tile_width, tile_height],
            )?)?;
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
                    tile.encode_processing(device, &mut encoder, &input)?;
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
