//! Original-first regional processing and bounded affine reconstruction.
use super::{
    WorkBudget,
    raster::{CameraProgram, Image, Request as RasterRequest},
};
use crate::{
    Error,
    acquisition::{OriginalBatch, SourceRead},
    gpu::{GpuFlatField, ProcessingInput, SourceGeometry, SubmissionGate},
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
    let start = std::time::Instant::now();
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
    let pipeline = input.pixel_pipeline()?;
    let tile_size = [512, 128];
    let batch = OriginalBatch::new(scan.area, demand.area, &pipeline, tile_size)?;
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
    let mut extracted = None;
    let mut image: Option<Image> = None;
    let mut sampling = std::time::Duration::ZERO;
    let mut gpu = std::time::Duration::ZERO;
    for unit in batch.units() {
        if cancelled() {
            return Ok(None);
        }
        let tick = std::time::Instant::now();
        let unit = unit?;
        let SourceRead::Originals(read) = unit.plan.source_read() else {
            unreachable!()
        };
        let size = [read.width, read.height];
        WorkBudget::for_device(device).require_source(size)?;
        Image::require(device, size, OutputReference::DisplayReferred(target))?;
        let pixels = Image::read_originals(scan, unit.plan.source_read())?;
        if image.as_ref().is_none_or(|image| image.size != size) {
            drop(image.take());
            let program = match input.pipeline() {
                Some(pipeline) => CameraProgram::Interpreted(pipeline.input_program()),
                None => CameraProgram::Matrix(scan.camera_to_ap1),
            };
            image = Some(Image::from_pixels(
                device,
                renderer,
                size,
                program,
                OutputReference::DisplayReferred(target),
                input.look(),
                &pixels,
            )?);
        } else {
            image
                .as_ref()
                .unwrap()
                .source
                .write_continuous(queue, &pixels)?;
        }
        let image = image.as_mut().unwrap();
        image.source.set_flat_field(
            field.clone(),
            SourceGeometry::original_tile([read.x, read.y, read.width, read.height], size)?,
        )?;
        sampling += tick.elapsed();
        let tick = std::time::Instant::now();
        let origin = [unit.output.x - demand.area.x, unit.output.y - demand.area.y];
        let extraction = unit.offset != [0; 2] || size != [unit.output.width, unit.output.height];
        let tile = if extraction {
            extracted.get_or_insert_with(|| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("valid original cells for reconstruction"),
                    size: wgpu::Extent3d {
                        width: tile_size[0],
                        height: tile_size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba32Float,
                    usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
            })
        } else {
            &image.texture
        };
        for (index, window) in average.windows(tile, origin).into_iter().enumerate() {
            if cancelled() {
                return Ok(None);
            }
            let mut encoder = device.create_command_encoder(&Default::default());
            let prepared =
                if index == 0 {
                    let prepared = RasterRequest::new(&image.source, input, Some(&unit.plan))
                        .encode(renderer, device, &mut encoder, &image.working)?;
                    image.render(renderer, device, &mut encoder, target)?;
                    if extraction {
                        encoder.copy_texture_to_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture: &image.texture,
                                mip_level: 0,
                                origin: wgpu::Origin3d {
                                    x: unit.offset[0],
                                    y: unit.offset[1],
                                    z: 0,
                                },
                                aspect: wgpu::TextureAspect::All,
                            },
                            tile.as_image_copy(),
                            wgpu::Extent3d {
                                width: unit.output.width,
                                height: unit.output.height,
                                depth_or_array_layers: 1,
                            },
                        );
                    }
                    prepared
                } else {
                    None
                };
            average.accumulate_window(device, &mut encoder, tile, origin, window);
            let submission = gate.submit(queue, [encoder.finish()]);
            if let Some(prepared) = &prepared {
                prepared.submitted();
            }
            wait_for(device, submission)?;
        }
        gpu += tick.elapsed();
    }
    if cancelled() {
        return Ok(None);
    }
    let mut encoder = device.create_command_encoder(&Default::default());
    average.finish(device, &mut encoder, output);
    let look_check = input
        .look()
        .map(|_| renderer.check_look_image(device, &mut encoder, output));
    let submission = gate.submit(queue, [encoder.finish()]);
    if let Some(check) = &look_check {
        check.begin(|| {});
    }
    wait_for(device, submission)?;
    if let Some(check) = &look_check {
        check
            .result()
            .ok_or_else(|| Error::Gpu("look validation did not complete".into()))??;
    }
    if cancelled() {
        return Ok(None);
    }
    log::debug!(
        source_width = demand.area.width, source_height = demand.area.height,
        output_width = demand.output[0], output_height = demand.output[1], target:?,
        seconds = start.elapsed().as_secs_f64(), sampling_seconds = sampling.as_secs_f64(), gpu_seconds = gpu.as_secs_f64();
        "settled_preview"
    );
    Ok(Some(Completed { look_check }))
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
