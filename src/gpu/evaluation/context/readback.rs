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
        gate: &crate::gpu::SubmissionGate,
        evaluation: &Evaluation,
        cancelled: impl Fn() -> bool,
        mut consume: impl FnMut(SourceRegion, &[[f32; 4]]) -> Result<(), Error>,
    ) -> Result<Option<Stats>, Error> {
        let mut executor = self.executor();
        let mut total = Stats::default();
        let mut readback: Option<wgpu::Buffer> = None;
        let mut plans = evaluation.plans();

        loop {
            if cancelled() {
                return Ok(None);
            }
            let Some(plan) = plans.next() else {
                break;
            };
            let plan = plan?;
            Executor::admit(&self.device.limits(), &plan)?;
            if plan.tiles.len() != 1 {
                return Err(Error::InvalidParameter(
                    "host delivery requires one bounded evaluation block",
                ));
            }

            let bytes = u64::from(plan.region.width) * u64::from(plan.region.height) * 16;
            let tile = &plan.tiles[0];
            if tile.fragment.resources[tile.result.resource.0].bytes() < bytes {
                return Err(Error::InvalidParameter(
                    "host float delivery requires a complete RGBA buffer",
                ));
            }

            if readback.as_ref().is_none_or(|buffer| buffer.size() < bytes) {
                readback = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("bounded image-query readback"),
                    size: bytes,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }));
                total.buffer_allocations += 1;
            }
            let buffer = readback.as_ref().unwrap();
            let (send, receive) = mpsc::channel();

            let Some(stats) = executor.run_sink_with(
                &self.device,
                queue,
                gate,
                &plan,
                &cancelled,
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

            total.submissions += stats.submissions;
            total.source_cells += stats.source_cells;
            total.buffer_allocations += stats.buffer_allocations;
            total.texture_allocations += stats.texture_allocations;
            total.initializations += stats.initializations;
            total.encoded_device_peak = total.encoded_device_peak.max(stats.encoded_device_peak);
            total.scratch_bytes = total.scratch_bytes.max(stats.scratch_bytes);
        }

        Ok((!cancelled()).then_some(total))
    }
}
