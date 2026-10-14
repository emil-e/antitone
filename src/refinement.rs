use crate::{
    Error,
    gpu::{GpuFlatField, ProcessingParameters, SourceGeometry},
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
    parameters: ProcessingParameters,
    target: OutputTarget,
    (generation, expected): (&AtomicU64, u64),
    view: ViewRequest,
) -> Result<Option<Arc<GpuImage>>, Error> {
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
    let tile_width = 1024;
    let tile_height = 256;
    let zeros = vec![[0; 4]; tile_width as usize * tile_height as usize];
    let mut tile = pollster::block_on(GpuImage::new_shared(
        renderer,
        device,
        tile_width,
        tile_height,
        &zeros,
        scan.camera_to_ap1,
        target,
    ))?;
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
            tile.write_source(queue, &pixels)?;
            if let Some(field) = &field {
                tile.set_flat_field(
                    Some(field.clone()),
                    SourceGeometry::new(
                        [view.area.x + x, view.area.y + y, tile_width, tile_height],
                        [tile_width, tile_height],
                    )?,
                )?;
            }
            let mut encoder = device.create_command_encoder(&Default::default());
            tile.encode(device, &mut encoder, parameters);
            average.accumulate(device, &mut encoder, &tile.linear, [x, y]);
            wait_for(device, submissions.submit(queue, [encoder.finish()]))?;
            gpu += tick.elapsed();
        }
    }
    let mut encoder = device.create_command_encoder(&Default::default());
    average.finish(device, &mut encoder, &output.linear);
    submissions.submit(queue, [encoder.finish()]);
    println!(
        "settled_preview source={}x{} output={}x{} target={target:?} seconds={:.3} sampling_seconds={:.3} gpu_seconds={:.3}",
        view.area.width,
        view.area.height,
        width,
        height,
        start.elapsed().as_secs_f64(),
        sampling.as_secs_f64(),
        gpu.as_secs_f64()
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
