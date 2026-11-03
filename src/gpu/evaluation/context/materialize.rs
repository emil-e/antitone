//! Produce source-bearing image requests independently of their consuming command encoders.
use super::*;
use wgpu::util::DeviceExt;

#[cfg(test)]
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

impl Context {
    pub fn materialize(
        &self,
        queue: &wgpu::Queue,
        gate: &crate::gpu::submission::SubmissionGate,
        scope: &Materializations,
        evaluation: Evaluation,
        format: ImageFormat,
        backing: ImageBacking,
    ) -> Result<ImageFuture<ImageLayout>, Error> {
        self.materialize_into(queue, gate, scope, evaluation, (format, backing), None)
    }

    pub(in crate::gpu::evaluation) fn materialize_into(
        &self,
        queue: &wgpu::Queue,
        gate: &crate::gpu::submission::SubmissionGate,
        scope: &Materializations,
        evaluation: Evaluation,
        storage: (ImageFormat, ImageBacking),
        admission: Option<Arc<cache::Lease>>,
    ) -> Result<ImageFuture<ImageLayout>, Error> {
        let (format, backing) = storage;
        if !scope.belongs_to(self) {
            return Err(Error::InvalidParameter(
                "materializations belong to a different evaluation context",
            ));
        }
        let evaluation = evaluation.with_materializations(scope.clone());
        let image = evaluation.job.image();
        let key = image.identity().cloned().ok_or(Error::InvalidParameter(
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
        if let Some(image) = scope.find::<ImageLayout>(&key, |stored| stored.supplies(layout)) {
            return Ok(image);
        }
        let context = self.clone();
        let products = scope.clone();
        let queue = queue.clone();
        let gate = gate.clone();
        Ok(
            scope.obtain(key, Arc::new(layout), bytes, move || async move {
                let progress = Arc::new(futures_util::task::AtomicWaker::new());
                let wake = progress.clone();
                let mut job = Box::pin(crate::background::run(move |stop| {
                    if stop.is_cancelled() {
                        return Err(Error::Cancelled);
                    }
                    let image = if let Some(source) = source {
                        match backing {
                            ImageBacking::Buffer => {
                                let upload = read_source(source.as_ref(), layout.region, format)?;
                                if stop.is_cancelled() {
                                    return Err(Error::Cancelled);
                                }
                                GpuImage::new(context.device.create_buffer_init(
                                    &wgpu::util::BufferInitDescriptor {
                                        label: Some("materialized source image"),
                                        contents: upload.bytes(),
                                        usage: wgpu::BufferUsages::STORAGE
                                            | wgpu::BufferUsages::COPY_SRC
                                            | wgpu::BufferUsages::COPY_DST,
                                    },
                                ))
                            }
                            ImageBacking::Texture => {
                                let texture =
                                    context.device.create_texture(&wgpu::TextureDescriptor {
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
                                    });
                                // Bound CPU and queued upload storage independently of image size.
                                let row_bytes =
                                    u64::from(layout.region.width) * format.pixel_bytes();
                                let rows = ((4 << 20) / row_bytes)
                                    .max(1)
                                    .min(u64::from(layout.region.height))
                                    as u32;
                                let mut submitted = false;
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
                                        let upload = read_source(source.as_ref(), region, format)?;
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
                                        uploads.push_back(gate.submit(&queue, std::iter::empty()));
                                        submitted = true;
                                        if uploads.len() == 2 {
                                            context
                                                .device
                                                .poll(wgpu::PollType::Wait {
                                                    submission_index: uploads.pop_front(),
                                                    timeout: Some(std::time::Duration::from_secs(
                                                        10,
                                                    )),
                                                })
                                                .map_err(|error| Error::Gpu(error.to_string()))?;
                                        }
                                        wake.wake();
                                    }

                                    Ok(())
                                })();
                                // Queued uploads own storage until completion, including cancellation
                                // and a later read failure. Keep the source admission charged through it.
                                if submitted {
                                    context.submit(
                                        &queue,
                                        &gate,
                                        context.device.create_command_encoder(&Default::default()),
                                        || false,
                                    )?;
                                }
                                transferred?;
                                GpuImage::texture(texture)
                            }
                        }
                    } else {
                        let pixels = context.device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("materialized image"),
                            size: bytes,
                            usage: wgpu::BufferUsages::STORAGE
                                | wgpu::BufferUsages::COPY_SRC
                                | wgpu::BufferUsages::COPY_DST,
                            mapped_at_creation: false,
                        });
                        let worker = Production {
                            context: &context,
                            queue: &queue,
                            gate: &gate,
                            scope: &products,
                            budget: evaluation.job.budget(),
                            layout,
                            pixels: &pixels,
                            cancelled: &|| stop.is_cancelled(),
                            progress: &|| wake.wake(),
                        };
                        worker.produce(&evaluation)?;
                        GpuImage::new(pixels)
                    };
                    if let Some(admission) = admission {
                        image.admit(admission);
                    }
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
            }),
        )
    }
}
