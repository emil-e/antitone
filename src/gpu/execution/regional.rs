//! Original-source compositions with bounded lazy evaluation and affine reconstruction.

use crate::{
    Error,
    gpu::{GpuFlatField, ProcessingInput, SubmissionGate, evaluation},
    renderer::GpuRenderer,
    rendering::{OutputReference, OutputTarget},
    resample::AreaAverage,
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
    pub look_check: Option<Arc<crate::gpu::look::Check>>,
}

pub(crate) fn execute(
    renderer: &GpuRenderer,
    device: &wgpu::Device,
    (queue, gate): (&wgpu::Queue, &SubmissionGate),
    request: Request<'_>,
    output: &wgpu::Texture,
    cancelled: impl Fn() -> bool,
) -> Result<Option<Completed>, Error> {
    if cancelled() {
        return Ok(None);
    }
    request.validate()?;
    let Request {
        scan,
        field,
        input,
        target,
        demand,
    } = request;
    if output.width() != demand.output[0] || output.height() != demand.output[1] {
        return Err(Error::InvalidParameter(
            "regional output differs from demand",
        ));
    }
    let started = std::time::Instant::now();
    let root = evaluation::original_chain(
        device,
        renderer,
        scan,
        field,
        input,
        OutputReference::DisplayReferred(target),
        &input.pixel_pipeline()?,
    )?;
    let tile_size = [512, 128];
    let region = evaluation::Region {
        bounds: demand.area,
        grid: root.domain.grid,
    };
    let evaluation = renderer.evaluation.request(
        root,
        region,
        evaluation::Budget {
            tile: tile_size,
            scratch_bytes: (32 << 20).min(device.limits().max_buffer_size),
        },
    )?;
    let average = if let Some(frame) = demand.frame {
        AreaAverage::new_affine_shared(device, renderer.average.clone(), frame)?
    } else {
        AreaAverage::new_shared(
            device,
            renderer.average.clone(),
            [demand.area.width, demand.area.height],
            demand.output,
        )?
    };
    let tile = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("valid original output for affine reconstruction"),
        size: wgpu::Extent3d {
            width: tile_size[0],
            height: tile_size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let mut executor = renderer.evaluation.executor();
    for plan in evaluation.plans() {
        if cancelled() {
            return Ok(None);
        }
        let plan = plan?;
        let region = plan.region;
        let origin = [region.x - demand.area.x, region.y - demand.area.y];
        let windows = average.windows(&tile, origin);
        let mut windows = windows.into_iter();
        let Some(first) = windows.next() else {
            continue;
        };
        let completed = executor.run_sink(
            device,
            queue,
            gate,
            &plan,
            &cancelled,
            |encoder, image, pixels| {
                evaluation::acquire::copy_sized(
                    &renderer.evaluation,
                    encoder,
                    pixels,
                    image.size(),
                    &tile,
                )?;
                average.accumulate_window(device, encoder, &tile, origin, first)?;
                Ok(())
            },
        )?;
        if completed.is_none() {
            return Ok(None);
        }
        for window in windows {
            if cancelled() {
                return Ok(None);
            }
            let mut encoder = device.create_command_encoder(&Default::default());
            average.accumulate_window(device, &mut encoder, &tile, origin, window)?;
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
    let mut encoder = device.create_command_encoder(&Default::default());
    average.finish(device, &mut encoder, output)?;
    let look_check = input
        .look()
        .map(|_| renderer.check_look_image(device, &mut encoder, output));
    if renderer
        .evaluation
        .submit_with(queue, gate, encoder, &cancelled, || {
            if let Some(check) = &look_check {
                check.begin(|| {});
            }
        })?
        .is_none()
    {
        return Ok(None);
    }
    if let Some(check) = &look_check {
        check
            .result()
            .ok_or_else(|| Error::Gpu("look validation did not complete".into()))??;
    }
    if cancelled() {
        return Ok(None);
    }
    log::debug!(
        source_width=demand.area.width,source_height=demand.area.height,
        output_width=demand.output[0],output_height=demand.output[1],target:?,
        seconds=started.elapsed().as_secs_f64();"settled_preview"
    );
    Ok(Some(Completed { look_check }))
}
