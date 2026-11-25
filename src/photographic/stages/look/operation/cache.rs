//! Weak device-local sharing; active consumers own immutable LUT resource leases.
use super::{LookId, Lut, Lut3D};
use crate::Error;
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};

#[derive(Default)]
pub(crate) struct Cache {
    luts: HashMap<LookId, Weak<Lut>>,
}
impl Cache {
    pub fn get(&mut self, device: &wgpu::Device, lut: &Lut3D) -> Result<(Arc<Lut>, bool), Error> {
        self.luts.retain(|_, v| v.strong_count() != 0);
        if let Some(value) = self.luts.get(&lut.id()).and_then(Weak::upgrade) {
            return Ok((value, false));
        }
        let value = Lut::new(device, lut)?;
        self.luts.insert(lut.id(), Arc::downgrade(&value));
        Ok((value, true))
    }
    #[cfg(test)]
    pub fn live(&self) -> usize {
        self.luts.values().filter(|v| v.strong_count() != 0).count()
    }
}
