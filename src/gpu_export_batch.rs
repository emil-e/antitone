//! At most two already-needed original misses share one GPU read.
use super::{Error, reader::TileLoader};
use std::collections::VecDeque;

type Origin = [u32; 2];
type Pixel = [f64; 4];

pub(super) struct PairLoader<L> {
    load: L,
    paired: bool,
    pixels: usize,
    misses: VecDeque<Origin>,
    pending: Option<(Origin, Vec<Pixel>)>,
    pub reads: u64,
    #[cfg(test)]
    pub dispatches: u64,
}

impl<L> PairLoader<L> {
    pub fn new(load: L, pixels: usize, paired: bool) -> Self {
        Self {
            load,
            paired,
            pixels,
            misses: VecDeque::new(),
            pending: None,
            reads: 0,
            #[cfg(test)]
            dispatches: 0,
        }
    }
}

impl<L: FnMut(&[Origin]) -> Result<Vec<Pixel>, Error>> TileLoader for PairLoader<L> {
    fn prepare(&mut self, misses: impl Iterator<Item = Origin>) -> Result<(), Error> {
        if self.pending.is_some() || !self.misses.is_empty() {
            return Err(Error::Export("unconsumed original batch".into()));
        }
        if self.paired {
            self.misses.extend(misses);
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<(), Error> {
        if self.pending.is_some() || !self.misses.is_empty() {
            return Err(Error::Export("unconsumed original batch".into()));
        }
        Ok(())
    }

    fn load(&mut self, origin: Origin) -> Result<Vec<Pixel>, Error> {
        if let Some((expected, pixels)) = self.pending.take() {
            if origin != expected {
                return Err(Error::Export(
                    "original batch order differs from plan".into(),
                ));
            }
            return Ok(pixels);
        }
        let second = if let Some(expected) = self.misses.pop_front() {
            if origin != expected {
                return Err(Error::Export(
                    "original batch order differs from plan".into(),
                ));
            }
            self.misses.pop_front()
        } else {
            None
        };
        let origins = [origin, second.unwrap_or(origin)];
        let count = if second.is_some() { 2 } else { 1 };
        self.reads += count as u64;
        #[cfg(test)]
        {
            self.dispatches += 1;
        }
        let mut pixels = (self.load)(&origins[..count])?;
        let expected = self
            .pixels
            .checked_mul(count)
            .ok_or(Error::NumericalOverflow)?;
        if pixels.len() != expected {
            return Err(Error::InvalidParameter(
                "invalid original export batch length",
            ));
        }
        if let Some(second) = second {
            self.pending = Some((second, pixels.split_off(self.pixels)));
        }
        Ok(pixels)
    }
}

#[cfg(test)]
#[path = "gpu_export_batch_tests.rs"]
mod tests;
