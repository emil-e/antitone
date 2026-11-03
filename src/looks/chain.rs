//! Ordered colour-treatment assets; metadata does not affect numeric identity.
use super::{LookAsset, LookId};
use std::sync::Arc;

#[derive(Clone, Debug, Default)]
pub struct LutChain {
    pub lut: Option<Arc<LookAsset>>,
    pub film_and_paper: Option<Arc<LookAsset>>,
}

impl LutChain {
    pub fn assets(&self) -> impl Iterator<Item = &Arc<LookAsset>> {
        self.lut.iter().chain(self.film_and_paper.iter())
    }

    pub fn id(&self) -> Option<LookId> {
        match (&self.lut, &self.film_and_paper) {
            (None, None) => None,
            (Some(asset), None) | (None, Some(asset)) => Some(asset.lut().id()),
            (Some(first), Some(second)) => {
                let mut hash = blake3::Hasher::new();
                hash.update(b"ordered-lut-chain-v1");
                hash.update(&first.lut().id().bytes());
                hash.update(&second.lut().id().bytes());
                Some(LookId(*hash.finalize().as_bytes()))
            }
        }
    }

    pub fn retained_bytes(&self) -> usize {
        self.assets().map(|asset| asset.retained_bytes()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.id().is_none()
    }
}

impl PartialEq for LutChain {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl From<Option<Arc<LookAsset>>> for LutChain {
    fn from(lut: Option<Arc<LookAsset>>) -> Self {
        Self {
            lut,
            film_and_paper: None,
        }
    }
}
