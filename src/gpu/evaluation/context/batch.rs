//! Related image roots share production without introducing another reuse authority.
use super::*;

impl Context {
    pub(crate) fn materialize_many(
        &self,
        queue: &wgpu::Queue,
        gate: &crate::gpu::submission::SubmissionGate,
        scope: &Materializations,
        evaluations: Vec<Evaluation>,
    ) -> Result<Vec<ImageFuture<ImageLayout>>, Error> {
        if !scope.belongs_to(self) {
            return Err(Error::InvalidParameter(
                "materializations belong to a different evaluation context",
            ));
        }
        let limits = self.device.limits();
        let mut requests = Vec::new();
        let mut jobs = Vec::new();
        for evaluation in evaluations {
            let evaluation = evaluation.with_materializations(scope.clone());
            let image = evaluation.job.image();
            let key = image.identity().cloned().ok_or(Error::InvalidParameter(
                "materialization requires captured image content",
            ))?;
            let layout = ImageLayout {
                domain: image.domain,
                region: evaluation.job.region(),
                format: ImageFormat::Rgba32Float,
                backing: ImageBacking::Buffer,
            };
            if layout.bytes() > limits.max_buffer_size
                || layout.bytes() > limits.max_storage_buffer_binding_size
            {
                return Err(Error::InvalidParameter(
                    "image materialization exceeds GPU storage limits",
                ));
            }
            requests.push((key, Arc::new(layout), layout.bytes()));
            jobs.push((evaluation, layout));
        }
        let (context, queue, gate, scope_owned) =
            (self.clone(), queue.clone(), gate.clone(), scope.clone());
        Ok(scope.obtain_many(requests, move |missing| async move {
            let progress = Arc::new(futures_util::task::AtomicWaker::new());
            let wake = progress.clone();
            let mut job = Box::pin(crate::background::run(move |stop| {
                if stop.is_cancelled() {
                    return Err(Error::Cancelled);
                }
                let selected: Vec<_> = missing.into_iter().map(|i| &jobs[i]).collect();
                let pixels: Vec<_> = selected
                    .iter()
                    .map(|(_, layout)| {
                        context.device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("materialized image"),
                            size: layout.bytes(),
                            usage: wgpu::BufferUsages::STORAGE
                                | wgpu::BufferUsages::COPY_SRC
                                | wgpu::BufferUsages::COPY_DST,
                            mapped_at_creation: false,
                        })
                    })
                    .collect();
                let cancelled = || stop.is_cancelled();
                let notify = || wake.wake();
                if cancelled() {
                    return Err(Error::Cancelled);
                }
                let outputs: Vec<_> = selected
                    .iter()
                    .zip(&pixels)
                    .map(|((evaluation, layout), pixels)| {
                        (
                            evaluation.job.image(),
                            Production {
                                context: &context,
                                queue: &queue,
                                gate: &gate,
                                scope: &scope_owned,
                                budget: evaluation.job.budget(),
                                layout: *layout,
                                pixels,
                                cancelled: &cancelled,
                                progress: &notify,
                            },
                        )
                    })
                    .collect();
                let stored = outputs.iter().try_fold(false, |stored, (image, worker)| {
                    Ok::<_, Error>(
                        stored
                            || super::super::materialized::Stored::find(
                                Some(&scope_owned),
                                &[],
                                image,
                                worker.layout.region,
                            )?
                            .is_some(),
                    )
                })?;
                let handled = if !stored
                    && let Some((image, _)) = outputs.first()
                    && let Some(producer) = image
                        .operation
                        .sample_consumer()
                        .and_then(|consumer| consumer.production())
                {
                    producer.produce_many(&outputs)?
                } else {
                    None
                };
                if handled.is_none() {
                    for ((_, worker), (evaluation, _)) in outputs.iter().zip(&selected) {
                        worker.produce(evaluation)?;
                    }
                }
                if cancelled() {
                    return Err(Error::Cancelled);
                }
                Ok(pixels.into_iter().map(GpuImage::new).collect())
            }));
            futures_util::future::poll_fn(|cx| {
                progress.register(cx.waker());
                std::future::Future::poll(job.as_mut(), cx)
            })
            .await
            .map_err(|_| Error::Gpu("image materialization worker panicked".into()))?
        }))
    }
}
