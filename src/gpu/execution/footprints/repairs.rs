//! Rebuild affected footprints from the complete repaired root, preserving untouched samples.

use super::*;
use crate::gpu::evaluation::{self as e, ImageBacking, ImageFormat, ImageLayout};

impl Acquired {
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn reconstruct_repairs(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        base: Arc<Self>,
        repairs: crate::gpu::repairs::Snapshot,
        previous: Option<Arc<Self>>,
        scope: &e::Materializations,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Arc<Self>>, Error> {
        if cancelled() {
            return Ok(None);
        }
        let context = base.renderer.evaluation.clone();
        context.require_device(device)?;
        if !scope.belongs_to(&context) {
            return Err(Error::InvalidParameter(
                "materializations belong to a different evaluation context",
            ));
        }
        if !base.spec.repairs.0.is_empty() {
            return Err(Error::Gpu(
                "repair reconstruction requires an unrepaired neutral base".into(),
            ));
        }
        if repairs.0.is_empty() {
            return Ok(Some(base));
        }
        let base_layout = base.supplied()?.layout();
        let previous = previous.filter(|previous| {
            previous.basis.same_definition(&base.definition)
                && previous
                    .supplied()
                    .is_ok_and(|image| image.layout() == base_layout)
        });
        let empty = crate::gpu::repairs::Snapshot::default();
        let old = previous.as_ref().map_or(&empty, |p| &p.spec.repairs);
        let changed = repairs
            .0
            .iter()
            .filter(|p| !old.0.iter().any(|q| Arc::ptr_eq(p, q)))
            .chain(
                old.0
                    .iter()
                    .filter(|p| !repairs.0.iter().any(|q| Arc::ptr_eq(p, q))),
            );
        let changed = crate::gpu::repairs::Snapshot(changed.cloned().collect());
        let frame = base.spec.plan.output().processing_region(
            [
                f64::from(base_layout.region.x),
                f64::from(base_layout.region.y),
            ],
            [base_layout.region.width, base_layout.region.height],
        )?;
        let windows = frame.affected_windows(
            changed.0.iter().map(|patch| {
                let support = patch.evaluated.support();
                let origin = support.origin();
                let size = support.size();
                [origin[0], origin[1], size[0], size[1]].into()
            }),
            Default::default(),
        )?;
        let spec = base.spec.clone().with_repairs(repairs.clone());
        let definition = e::Photograph::repaired_reconstruction(&base.definition, repairs.clone())?;
        let layout = ImageLayout {
            domain: definition.domain(),
            region: base_layout.region,
            format: ImageFormat::Rgba32Float,
            backing: ImageBacking::Buffer,
        };
        let bytes = layout.bytes();
        if bytes > device.limits().max_buffer_size
            || bytes > device.limits().max_storage_buffer_binding_size
        {
            return Err(Error::InvalidParameter(
                "image materialization exceeds GPU storage limits",
            ));
        }
        let key = definition
            .identity()
            .cloned()
            .ok_or(Error::InvalidParameter(
                "materialization requires captured image content",
            ))?;
        let initial = previous.unwrap_or_else(|| base.clone());
        let (gpu, queue_owned, gate, products, root) = (
            device.clone(),
            queue.clone(),
            submissions.clone(),
            scope.clone(),
            definition.clone(),
        );
        let producer_base = base.clone();
        let future = scope.obtain(key, Arc::new(layout), bytes, move || async move {
            crate::background::run(move |stop| {
                if stop.is_cancelled() {
                    return Err(Error::Cancelled);
                }
                let pixels = float_buffer(&gpu, frame.output)?;
                let mut encoder = gpu.create_command_encoder(&Default::default());
                encoder.copy_buffer_to_buffer(&initial.pixels, 0, &pixels, 0, pixels.size());
                context
                    .submit(&queue_owned, &gate, encoder, || stop.is_cancelled())?
                    .ok_or(Error::Cancelled)?;
                for (origin, size) in windows {
                    if stop.is_cancelled() {
                        return Err(Error::Cancelled);
                    }
                    let region = e::CellRegion::new(
                        [
                            layout.region.x + origin[0] as i32,
                            layout.region.y + origin[1] as i32,
                        ],
                        size,
                    )
                    .validate()?;
                    let budget = e::Budget {
                        tile: size,
                        scratch_bytes: 512 << 20,
                    };
                    let evaluation = context.request(
                        root.clone(),
                        e::Region {
                            bounds: region,
                            grid: layout.domain.grid,
                        },
                        budget,
                    )?;
                    let rebuilt = float_buffer(&gpu, size)?;
                    // The owning materialization produces its windows as scratch, not another
                    // reservation of the same value (a whole-frame window can be the root).
                    e::Production {
                        context: &context,
                        queue: &queue_owned,
                        gate: &gate,
                        scope: &products,
                        budget,
                        layout: ImageLayout { region, ..layout },
                        pixels: &rebuilt,
                        cancelled: &|| stop.is_cancelled(),
                        progress: &|| {},
                    }
                    .produce(&evaluation)?;
                    let mut encoder = gpu.create_command_encoder(&Default::default());
                    e::acquire::repair_copy(
                        &context,
                        &mut encoder,
                        &pixels,
                        (&producer_base.pixels, ([0, 0], frame.output)),
                        frame,
                        (origin, size),
                        &changed,
                    )?;
                    e::acquire::repair_copy(
                        &context,
                        &mut encoder,
                        &pixels,
                        (&rebuilt, (origin, size)),
                        frame,
                        (origin, size),
                        &repairs,
                    )?;
                    context
                        .submit(&queue_owned, &gate, encoder, || stop.is_cancelled())?
                        .ok_or(Error::Cancelled)?;
                }
                if stop.is_cancelled() {
                    return Err(Error::Cancelled);
                }
                Ok(e::GpuImage::new(pixels))
            })
            .await
            .map_err(|_| Error::Gpu("repair reconstruction worker panicked".into()))?
        });
        let mut pending = Box::pin(future);
        let result = futures_util::future::poll_fn(|cx| {
            if cancelled() {
                return std::task::Poll::Ready(None);
            }
            std::future::Future::poll(pending.as_mut(), cx).map(Some)
        })
        .await;
        let image = match result {
            None => return Ok(None),
            Some(Err(error)) if matches!(error.as_ref(), Error::Cancelled) => return Ok(None),
            Some(Err(error)) => return Err(Error::Gpu(error.to_string())),
            Some(Ok(image)) => image,
        };
        if cancelled() {
            return Ok(None);
        }
        Ok(Some(Arc::new(Self {
            spec,
            basis: base.definition.clone(),
            definition,
            pixels: image.try_into()?,
            renderer: base.renderer.clone(),
        })))
    }
}
