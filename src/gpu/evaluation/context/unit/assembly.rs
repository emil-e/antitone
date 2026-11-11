//! Bounded ready-copy ownership, separate from calculating the next work unit.
use super::*;

#[cfg(all(test, feature = "editor"))]
mod tests;

#[derive(Default)]
pub(super) struct Copies {
    encoder: Option<wgpu::CommandEncoder>,
    images: Vec<CopyInput>,
    bytes: u64,
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
            self.bytes = 0;
            if (production.cancelled)() {
                return Err(Error::Cancelled);
            }
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
            let result =
                production
                    .context
                    .submit(production.queue, production.gate, encoder, || false);
            drop(images);
            result?.ok_or(Error::Cancelled)?;
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
        let bytes = rgba_bytes(output)?;
        if self
            .bytes
            .checked_add(bytes)
            .is_none_or(|total| total > production.budget.scratch_bytes)
        {
            self.flush(production)?;
        }
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
        self.bytes += bytes;
        // A single whole CPU result or independently held GPU value can exceed
        // this allowance; preserve its existing one-unit transfer behavior.
        if self.bytes >= production.budget.scratch_bytes {
            self.flush(production)?;
        }
        Ok(())
    }
}
