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
            requests.push((
                key,
                Arc::new(layout),
                layout.bytes(),
                evaluation.request_policy(),
            ));
            jobs.push((evaluation, layout));
        }
        let (context, queue, gate, scope_owned) =
            (self.clone(), queue.clone(), gate.clone(), scope.clone());
        Ok(scope.obtain_many_for(requests, move |missing| async move {
            let progress = Arc::new(futures_util::task::AtomicWaker::new());
            let wake = progress.clone();
            let mut job = Box::pin(crate::background::run(move |stop| {
                if stop.is_cancelled() {
                    return Err(Error::Cancelled);
                }
                let selected: Vec<_> = missing.into_iter().map(|i| &jobs[i]).collect();
                let working = HostWorkingSet::default();
                let cancelled = || stop.is_cancelled();
                let notify = || wake.wake();
                if let Some((evaluation, _)) = selected.first() {
                    let worker = Worker {
                        context: &context,
                        queue: &queue,
                        gate: &gate,
                        scope: &scope_owned,
                        working: &working,
                        budget: evaluation.job.budget(),
                        cancelled: &cancelled,
                        progress: &notify,
                    };
                    if let Some(images) = worker.materialize_units_many(&selected)? {
                        return Ok(images);
                    }
                }
                let mut images = Vec::with_capacity(selected.len());
                for (evaluation, layout) in selected {
                    if cancelled() {
                        return Err(Error::Cancelled);
                    }
                    let worker = Worker {
                        context: &context,
                        queue: &queue,
                        gate: &gate,
                        scope: &scope_owned,
                        working: &working,
                        budget: evaluation.job.budget(),
                        cancelled: &cancelled,
                        progress: &notify,
                    };
                    let image = worker.materialize_image(
                        evaluation,
                        *layout,
                        &mut WorkCounts::default(),
                    )?;
                    // Completed roots can supply later dependencies even while this related batch is running.
                    scope_owned.publish(
                        &Image {
                            identity: evaluation
                                .job
                                .image()
                                .output(layout.region)?
                                .identity()
                                .cloned()
                                .unwrap(),
                            descriptor: Arc::new(*layout),
                            image: image.clone(),
                        },
                        evaluation.retain,
                    );
                    images.push(image);
                }
                if cancelled() {
                    return Err(Error::Cancelled);
                }
                Ok(images)
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
