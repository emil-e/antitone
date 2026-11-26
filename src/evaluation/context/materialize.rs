//! Produce source-bearing image requests independently of their consuming command encoders.
use super::*;
use crate::gpu::allocation::DeviceAllocationExt;
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

#[cfg(all(test, feature = "editor"))]
mod tests;

enum Upload {
    Codes(Vec<[u16; 4]>),
    Continuous(Vec<[f32; 4]>),
}
impl Upload {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Codes(pixels) => bytemuck::cast_slice(pixels),
            Self::Continuous(pixels) => bytemuck::cast_slice(pixels),
        }
    }
}

fn read_source(
    source: &dyn Source,
    region: CellRegion,
    format: ImageFormat,
) -> Result<Upload, Error> {
    let upload = match format {
        ImageFormat::Rgba16Uint => Upload::Codes(source.read_codes(region)?),
        ImageFormat::Rgba32Float => {
            let values = source.read(region)?;
            if !values.iter().fold(true, |finite, pixel| {
                pixel
                    .iter()
                    .fold(finite, |finite, x| finite & x.is_finite())
            }) {
                return Err(Error::InvalidParameter(
                    "source materialization contains non-finite values",
                ));
            }
            Upload::Continuous(values)
        }
    };
    let expected = u64::from(region.width) * u64::from(region.height) * format.pixel_bytes();
    if upload.bytes().len() as u64 != expected {
        return Err(Error::InvalidParameter(
            "source differs from admitted materialization layout",
        ));
    }
    Ok(upload)
}

fn wait_for_upload(device: &wgpu::Device, done: mpsc::Receiver<()>) -> Result<(), Error> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        // A blocking poll holds a device lock needed by surface presentation.
        device
            .poll(wgpu::PollType::Poll)
            .map_err(|error| Error::Gpu(error.to_string()))?;
        match done.recv_timeout(Duration::from_millis(1)) {
            Ok(()) => return Ok(()),
            Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() < deadline => {}
            Err(error) => return Err(Error::Gpu(format!("Texture upload completion: {error}"))),
        }
    }
}

impl Context {
    pub fn materialize(
        &self,
        queue: &crate::gpu::submission::Queue,
        scope: &Materializations,
        evaluation: Evaluation,
        format: ImageFormat,
        backing: ImageBacking,
    ) -> Result<ImageFuture<ImageLayout>, Error> {
        if backing == ImageBacking::Host {
            return Err(Error::InvalidParameter(
                "host values are produced by declared CPU units",
            ));
        }
        if !scope.belongs_to(self) {
            return Err(Error::InvalidParameter(
                "materializations belong to a different evaluation context",
            ));
        }
        let evaluation = evaluation.with_materializations(scope.clone());
        let image = evaluation.job.image();
        let key = image
            .output(evaluation.job.region())?
            .identity()
            .cloned()
            .ok_or(Error::InvalidParameter(
                "materialization requires captured image content",
            ))?;
        let layout = ImageLayout {
            domain: image.domain,
            region: evaluation.job.region(),
            format,
            backing,
        };
        let bytes = layout.bytes();
        let limits = self.device.limits();
        if (backing == ImageBacking::Buffer
            && (bytes > limits.max_buffer_size || bytes > limits.max_storage_buffer_binding_size))
            || (backing == ImageBacking::Texture
                && (layout.region.width > limits.max_texture_dimension_2d
                    || layout.region.height > limits.max_texture_dimension_2d))
        {
            return Err(Error::InvalidParameter(
                "image materialization exceeds GPU storage limits",
            ));
        }
        let source = image.operation.source();
        if backing == ImageBacking::Texture && source.is_none() {
            return Err(Error::InvalidParameter(
                "texture materialization requires an acquired source image",
            ));
        }
        if format == ImageFormat::Rgba16Uint
            && source
                .as_ref()
                .is_none_or(|source| source.storage_format() != format)
        {
            return Err(Error::InvalidParameter(
                "image has no lossless native camera-code representation",
            ));
        }
        scope.requested(image, layout.region);
        if let Some(image) = scope.find_for::<ImageLayout>(
            &key,
            |stored| stored.supplies(layout),
            evaluation.request_policy(),
        ) {
            return Ok(image);
        }
        let context = self.clone();
        let products = scope.clone();
        let queue = queue.clone();

        Ok(scope.obtain_for(
            key,
            Arc::new(layout),
            bytes,
            evaluation.request_policy(),
            move || async move {
                let progress = Arc::new(futures_util::task::AtomicWaker::new());
                let wake = progress.clone();
                let mut job = Box::pin(crate::background::run(move |stop| {
                    if stop.is_cancelled() {
                        return Err(Error::Cancelled);
                    }
                    let image = if let Some(source) = source {
                        match backing {
                            ImageBacking::Host => unreachable!(),
                            ImageBacking::Buffer => {
                                let upload = read_source(source.as_ref(), layout.region, format)?;
                                if stop.is_cancelled() {
                                    return Err(Error::Cancelled);
                                }
                                ImageValue::new(context.device.try_create_buffer_init(
                                    &wgpu::util::BufferInitDescriptor {
                                        label: Some("materialized source image"),
                                        contents: upload.bytes(),
                                        usage: wgpu::BufferUsages::STORAGE
                                            | wgpu::BufferUsages::COPY_SRC
                                            | wgpu::BufferUsages::COPY_DST,
                                    },
                                )?)
                            }
                            ImageBacking::Texture => {
                                let texture = context.device.try_create_texture(
                                    &wgpu::TextureDescriptor {
                                        label: Some("materialized source image"),
                                        size: wgpu::Extent3d {
                                            width: layout.region.width,
                                            height: layout.region.height,
                                            depth_or_array_layers: 1,
                                        },
                                        mip_level_count: 1,
                                        sample_count: 1,
                                        dimension: wgpu::TextureDimension::D2,
                                        format: match format {
                                            ImageFormat::Rgba16Uint => {
                                                wgpu::TextureFormat::Rgba16Uint
                                            }
                                            ImageFormat::Rgba32Float => {
                                                wgpu::TextureFormat::Rgba32Float
                                            }
                                        },
                                        usage: wgpu::TextureUsages::TEXTURE_BINDING
                                            | wgpu::TextureUsages::COPY_DST
                                            | wgpu::TextureUsages::COPY_SRC,
                                        view_formats: &[],
                                    },
                                )?;
                                // Bound CPU and queued upload storage independently of image size.
                                let row_bytes =
                                    u64::from(layout.region.width) * format.pixel_bytes();
                                let rows = ((4 << 20) / row_bytes)
                                    .max(1)
                                    .min(u64::from(layout.region.height))
                                    as u32;
                                let started = Instant::now();
                                let mut read_time = Duration::ZERO;
                                let mut queue_time = Duration::ZERO;
                                let mut uploads = std::collections::VecDeque::new();
                                let transferred = (|| -> Result<(), Error> {
                                    for y in (0..layout.region.height).step_by(rows as usize) {
                                        if stop.is_cancelled() {
                                            break;
                                        }
                                        let region = CellRegion {
                                            y: layout.region.y + y as i32,
                                            height: rows.min(layout.region.height - y),
                                            ..layout.region
                                        };
                                        let reading = Instant::now();
                                        let upload = read_source(source.as_ref(), region, format)?;
                                        read_time += reading.elapsed();
                                        let submitting = Instant::now();
                                        queue.write_texture(
                                            wgpu::TexelCopyTextureInfo {
                                                texture: &texture,
                                                mip_level: 0,
                                                origin: wgpu::Origin3d { x: 0, y, z: 0 },
                                                aspect: wgpu::TextureAspect::All,
                                            },
                                            upload.bytes(),
                                            wgpu::TexelCopyBufferLayout {
                                                offset: 0,
                                                bytes_per_row: Some(row_bytes as u32),
                                                rows_per_image: Some(region.height),
                                            },
                                            wgpu::Extent3d {
                                                width: region.width,
                                                height: region.height,
                                                depth_or_array_layers: 1,
                                            },
                                        );
                                        let (send, done) = mpsc::sync_channel(1);
                                        queue.submit_with_completion(
                                            std::iter::empty(),
                                            move || {
                                                let _ = send.send(());
                                            },
                                        );
                                        queue_time += submitting.elapsed();
                                        if y == 0 {
                                            log::debug!(format:? = format, bytes = layout.bytes(); "texture_source_upload_started");
                                        }
                                        uploads.push_back(done);
                                        if uploads.len() == 2 {
                                            wait_for_upload(
                                                &context.device,
                                                uploads.pop_front().unwrap(),
                                            )?;
                                        }
                                        wake.wake();
                                    }

                                    Ok(())
                                })();
                                // Queued uploads own storage until completion, including cancellation
                                // and a later read failure.
                                for done in uploads {
                                    wait_for_upload(&context.device, done)?;
                                }
                                transferred?;
                                log::debug!(
                                    format:? = format,
                                    bytes = layout.bytes(),
                                    cancelled = stop.is_cancelled(),
                                    read_ms = read_time.as_secs_f64() * 1000.,
                                    submit_ms = queue_time.as_secs_f64() * 1000.,
                                    total_ms = started.elapsed().as_secs_f64() * 1000.;
                                    "texture_source_upload"
                                );
                                ImageValue::texture(texture)
                            }
                        }
                    } else {
                        let working = RunWorkingSet::default();
                        let worker = Worker {
                            context: &context,
                            queue: &queue,
                            scope: &products,
                            working: &working,
                            cancelled: &|| stop.is_cancelled(),
                            progress: &|| wake.wake(),
                        };
                        let image = worker.materialize_image(
                            &evaluation,
                            layout,
                            &mut WorkCounts::default(),
                        )?;
                        working.finish_gpu()?;
                        image
                    };
                    if stop.is_cancelled() {
                        return Err(Error::Cancelled);
                    }
                    Ok(image)
                }));
                futures_util::future::poll_fn(|cx| {
                    progress.register(cx.waker());
                    std::future::Future::poll(job.as_mut(), cx)
                })
                .await
                .map_err(|_| Error::Gpu("image materialization worker panicked".into()))?
            },
        ))
    }
}
