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
    repairs: &[Arc<crate::dust_repair::EvaluatedRepair>],
    parameters: ProcessingParameters,
    target: OutputTarget,
    (generation, expected): (&AtomicU64, u64),
    view: ViewRequest,
) -> Result<Option<Arc<GpuImage>>, Error> {
    refine_with_depth(
        renderer,
        device,
        (queue, submissions),
        (scan, field),
        repairs,
        parameters,
        target,
        (generation, expected),
        view,
        1,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn refine_with_depth(
    renderer: &Arc<crate::renderer::GpuRenderer>,
    device: &wgpu::Device,
    (queue, submissions): (&wgpu::Queue, &crate::platform::SubmissionGate),
    (scan, field): (&Scan, Option<Arc<GpuFlatField>>),
    repairs: &[Arc<crate::dust_repair::EvaluatedRepair>],
    parameters: ProcessingParameters,
    target: OutputTarget,
    (generation, expected): (&AtomicU64, u64),
    view: ViewRequest,
    depth: usize,
) -> Result<Option<Arc<GpuImage>>, Error> {
    if !(1..=3).contains(&depth) {
        return Err(Error::InvalidParameter("invalid refinement queue depth"));
    }
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
    let mut pending = std::collections::VecDeque::new();
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
            let (indices, values) =
                repair_patch(repairs, view.area, [x, y], [tile_width, tile_height]);
            let mut encoder = device.create_command_encoder(&Default::default());
            tile.encode_ap1_patch(device, &mut encoder, parameters, &indices, &values);
            average.accumulate(device, &mut encoder, &tile.linear, [x, y]);
            pending.push_back(submissions.submit(queue, [encoder.finish()]));
            // Queue order permits reuse of the tile buffers; only upload staging is outstanding.
            if pending.len() >= depth {
                wait_for(device, pending.pop_front().unwrap())?;
            }
            gpu += tick.elapsed();
        }
    }
    let tick = std::time::Instant::now();
    for submission in pending {
        wait_for(device, submission)?;
    }
    gpu += tick.elapsed();
    let mut encoder = device.create_command_encoder(&Default::default());
    average.finish(device, &mut encoder, &output.linear);
    submissions.submit(queue, [encoder.finish()]);
    println!(
        "settled_preview source={}x{} output={}x{} target={target:?} depth={depth} seconds={:.3} sampling_seconds={:.3} gpu_seconds={:.3}",
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
fn repair_patch(
    repairs: &[Arc<crate::dust_repair::EvaluatedRepair>],
    area: crate::raw::Rect,
    tile: [u32; 2],
    tile_size: [u32; 2],
) -> (Vec<u32>, Vec<[f32; 4]>) {
    let origin = [area.x + tile[0], area.y + tile[1]];
    let right = (origin[0] + tile_size[0]).min(area.x + area.width);
    let bottom = (origin[1] + tile_size[1]).min(area.y + area.height);
    let mut indices = Vec::new();
    let mut values = Vec::new();
    for repair in repairs {
        for (point, value) in repair.pixels() {
            if point[0] >= origin[0]
                && point[0] < right
                && point[1] >= origin[1]
                && point[1] < bottom
            {
                indices.push(point[0] - origin[0] + (point[1] - origin[1]) * tile_size[0]);
                values.push([value[0], value[1], value[2], 0.]);
            }
        }
    }
    (indices, values)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        dust_repair::{DonorChoice, Repair, RepairId, RepairOrigin, SupportMask},
        geometry::SourceRegion,
    };

    #[test]
    fn repair_patch_uses_tile_local_indices_and_ap1_values() {
        let size = [96, 96];
        let image = (0..size[0] * size[1])
            .map(|index| {
                let x = index % size[0];
                let y = index / size[0];
                [x as f32 * 0.001, y as f32 * 0.002, 1. + x as f32 * 0.003]
            })
            .collect::<Vec<_>>();
        let support = SupportMask::new([46, 46], [5, 5], vec![true; 25]).unwrap();
        let repair = Repair {
            id: RepairId(1),
            enabled: true,
            origin: RepairOrigin::Manual,
            support,
            donor: DonorChoice::Offset([-20, 0]),
        };
        let evaluated = crate::dust_repair::evaluate(
            &repair,
            SourceRegion {
                x: 0,
                y: 0,
                width: size[0],
                height: size[1],
            },
            |[x, y]| Ok(image[(y * size[0] + x) as usize]),
        )
        .unwrap();
        let repairs = [Arc::new(evaluated)];
        let (indices, values) = repair_patch(
            &repairs,
            crate::raw::Rect {
                x: 0,
                y: 0,
                width: size[0],
                height: size[1],
            },
            [32, 32],
            [32, 32],
        );
        assert_eq!(indices.len(), 25);
        assert_eq!(indices[0], 14 + 14 * 32);
        assert_eq!(indices[24], 18 + 18 * 32);
        assert_eq!(values[12][3], 0.);
        let repaired = repairs[0].pixel([48, 48], [0.; 3]);
        assert_eq!(&values[12][..3], repaired.as_slice());
    }
}
