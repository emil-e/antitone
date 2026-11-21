//! Bounded ready-copy ownership, separate from calculating the next work unit.
use super::*;

#[cfg(all(test, feature = "editor"))]
mod tests;

#[derive(Default)]
pub(super) struct Copies {
    encoder: Option<wgpu::CommandEncoder>,
    images: Vec<CopyInput>,
    pub counts: WorkCounts,
}
struct CopyInput {
    image: Arc<ImageValue>,
    output: CellRegion,
    overlap: CellRegion,
}
impl Copies {
    pub fn flush(&mut self, production: &Production<'_>) -> Result<(), Error> {
        if let Some(encoder) = self.encoder.take() {
            let images = std::mem::take(&mut self.images);
            if (production.cancelled)() {
                return Err(Error::Cancelled);
            }
            let gpu_only = images.iter().all(|input| input.image.as_buffer().is_some());
            let mut resources = Allocations::default();
            resources.buffer(production.pixels);
            for input in &images {
                resources.image(&input.image);
            }
            production.working.admit_gpu(production.context.device())?;
            // Queue writes cannot be withdrawn. Defer host staging until the batch
            // commits, then finish that bounded submission even if its owner retires.
            for input in &images {
                if let Some(values) = input.image.as_host() {
                    for y in input.overlap.y..input.overlap.bottom() {
                        let from = input.output.index([input.overlap.x, y]);
                        let to = production.layout.region.index([input.overlap.x, y]) as u64 * 16;
                        production.queue.write_buffer(
                            production.pixels,
                            to,
                            bytemuck::cast_slice(
                                &values[from..from + input.overlap.width as usize],
                            ),
                        );
                    }
                }
            }
            if gpu_only {
                production.working.submit_gpu(
                    production.context.device(),
                    production.queue,
                    encoder,
                    resources,
                    || {},
                )?;
            } else {
                production
                    .context
                    .submit(production.queue, encoder, || false)?
                    .ok_or(Error::Cancelled)?;
            }
            drop(images);
            if (production.cancelled)() {
                return Err(Error::Cancelled);
            }
            self.counts.submitted();
        }
        Ok(())
    }

    pub fn copy(
        &mut self,
        production: &Production<'_>,
        image: Arc<ImageValue>,
        output: CellRegion,
        overlap: CellRegion,
    ) -> Result<(), Error> {
        let encoder = self.encoder.get_or_insert_with(|| {
            production
                .context
                .device()
                .create_command_encoder(&Default::default())
        });
        if let Some(source) = image.as_buffer() {
            for y in overlap.y..overlap.bottom() {
                encoder.copy_buffer_to_buffer(
                    source,
                    output.index([overlap.x, y]) as u64 * 16,
                    production.pixels,
                    production.layout.region.index([overlap.x, y]) as u64 * 16,
                    u64::from(overlap.width) * 16,
                );
            }
        }
        self.images.push(CopyInput {
            image,
            output,
            overlap,
        });
        Ok(())
    }
}
