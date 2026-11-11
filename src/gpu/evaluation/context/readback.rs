//! Bounded host delivery of image blocks evaluated by the common worker.

use super::*;
use std::{sync::mpsc, time::Duration};

#[cfg(all(test, feature = "editor"))]
mod tests;

impl Context {
    /// Deliver row-major float pixels on the requested image grid, one bounded block at a time.
    /// The callback owns reduction/collection, not publication: discard partial results on error
    /// or cancellation. One readback block is retained in addition to evaluation scratch.
    /// This worker-side operation submits and waits; never call it during paint.
    pub fn read(
        &self,
        queue: &wgpu::Queue,
        gate: &crate::gpu::submission::SubmissionGate,
        evaluation: &Evaluation,
        cancelled: impl Fn() -> bool,
        consume: impl FnMut(CellRegion, &[[f32; 4]]) -> Result<(), Error>,
    ) -> Result<Option<WorkCounts>, Error> {
        self.read_in(
            (queue, gate),
            evaluation,
            (&cancelled, &|| {}),
            HostWorkingSet::default(),
            consume,
        )
    }

    pub(super) fn read_in(
        &self,
        (queue, gate): (&wgpu::Queue, &crate::gpu::submission::SubmissionGate),
        evaluation: &Evaluation,
        (cancelled, progress): (&dyn Fn() -> bool, &dyn Fn()),
        working: HostWorkingSet,
        mut consume: impl FnMut(CellRegion, &[[f32; 4]]) -> Result<(), Error>,
    ) -> Result<Option<WorkCounts>, Error> {
        let scope = evaluation
            .job
            .materializations()
            .cloned()
            .unwrap_or_else(|| self.materializations());
        let worker = Worker {
            context: self,
            queue,
            gate,
            scope: &scope,
            working: &working,
            budget: evaluation.job.budget(),
            cancelled,
            progress,
        };
        let mut total = WorkCounts::default();
        let host =
            match worker.host_image(evaluation.job.image(), evaluation.job.region(), &mut total) {
                Err(Error::Cancelled) if cancelled() => return Ok(None),
                value => value?,
            };
        if let Some((region, image)) = host {
            let values = image.as_host().expect("host image delivery");
            for block in tiled_units(evaluation.job.region(), evaluation.job.budget().tile)? {
                if cancelled() {
                    return Ok(None);
                }
                let mut pixels = Vec::with_capacity(block.width as usize * block.height as usize);
                for y in block.y..block.bottom() {
                    let start = region.index([block.x, y]);
                    pixels.extend_from_slice(&values[start..start + block.width as usize]);
                }
                consume(block, &pixels)?;
            }
            return Ok((!cancelled()).then_some(total));
        }
        let plans = super::production::admitted_plans(
            evaluation.clone(),
            evaluation.job.region(),
            evaluation.job.budget(),
            self.device.limits(),
            (cancelled, progress),
        )?;
        worker.read_plans(plans, total, consume)
    }
}

impl Worker<'_> {
    fn read_plans(
        &self,
        mut plans: impl Iterator<Item = Result<Plan, Error>>,
        mut total: WorkCounts,
        mut consume: impl FnMut(CellRegion, &[[f32; 4]]) -> Result<(), Error>,
    ) -> Result<Option<WorkCounts>, Error> {
        let (queue, gate) = (self.queue, self.gate);
        let (cancelled, progress) = (self.cancelled, self.progress);
        let mut executor = Executor::with_scope(self.context.programs.clone(), self.scope.clone())
            .with_working(self.working.clone());
        let mut readback: Option<wgpu::Buffer> = None;
        loop {
            if cancelled() {
                return Ok(None);
            }
            let Some(plan) = plans.next() else {
                break;
            };
            let plan = match plan {
                Err(Error::Cancelled) if cancelled() => return Ok(None),
                value => value?,
            };
            if plan.tiles.len() != 1 {
                return Err(Error::InvalidParameter(
                    "host delivery requires one bounded evaluation block",
                ));
            }

            let bytes = u64::from(plan.region.width) * u64::from(plan.region.height) * 16;
            let tile = &plan.tiles[0];
            let storage = &tile.fragment.resources[tile.result.resource.0];
            let available = match storage {
                Storage::ResidentBuffer(image) => {
                    image.as_buffer().map_or(0, |buffer| buffer.size())
                }
                storage => storage.bytes(),
            };
            if available < bytes {
                return Err(Error::InvalidParameter(
                    "host float delivery requires a complete RGBA buffer",
                ));
            }

            if readback.as_ref().is_none_or(|buffer| buffer.size() < bytes) {
                readback = Some(self.context.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("bounded image-query readback"),
                    size: bytes,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }));
                total.buffers_allocated(1);
            }
            let buffer = readback.as_ref().unwrap();
            let (send, receive) = mpsc::channel();

            let Some(stats) = self.context.run_sink(
                &mut executor,
                (queue, gate),
                &plan,
                (cancelled, progress),
                Sink {
                    encode: |encoder: &mut wgpu::CommandEncoder,
                             image: PlannedImage,
                             pixels: &wgpu::Buffer| {
                        if image.region != plan.region {
                            return Err(Error::InvalidParameter(
                                "host delivery differs from its evaluation block",
                            ));
                        }
                        if pixels.size() < bytes
                            || !pixels.usage().contains(wgpu::BufferUsages::COPY_SRC)
                        {
                            return Err(Error::InvalidParameter(
                                "host float delivery requires a readable RGBA buffer",
                            ));
                        }

                        encoder.copy_buffer_to_buffer(pixels, 0, buffer, 0, bytes);
                        Ok(())
                    },
                    submitted: || {
                        let send = send.clone();
                        buffer
                            .slice(..bytes)
                            .map_async(wgpu::MapMode::Read, move |result| {
                                let _ = send.send(result);
                            });
                    },
                },
            )?
            else {
                return Ok(None);
            };

            receive
                .recv_timeout(Duration::from_secs(2))
                .map_err(|_| Error::Gpu("image-query readback callback timed out".into()))?
                .map_err(|error| Error::Gpu(error.to_string()))?;
            let mapped = buffer
                .get_mapped_range(..bytes)
                .map_err(|error| Error::Gpu(error.to_string()))?;
            let result = consume(plan.region, bytemuck::cast_slice(&mapped));
            drop(mapped);
            buffer.unmap();
            result?;

            total.include(stats);
        }

        Ok((!cancelled()).then_some(total))
    }
}
