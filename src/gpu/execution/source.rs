//! Captured sample provenance; neither visit epochs nor publication tickets are content.

use crate::{gpu::ProcessingSource, scan::CameraIdentity};
use std::sync::{Arc, Weak};

#[derive(Clone)]
pub(crate) enum Identity {
    Camera(Arc<CameraIdentity>),
    Resident(Weak<ProcessingSource>),
    Proxy(Weak<Vec<[f32; 4]>>),
    Unbound(Arc<()>),
}

impl PartialEq for Identity {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Camera(a), Self::Camera(b)) => a == b,
            (Self::Resident(a), Self::Resident(b)) => a.ptr_eq(b),
            (Self::Proxy(a), Self::Proxy(b)) => a.ptr_eq(b),
            (Self::Unbound(a), Self::Unbound(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl Identity {
    pub fn camera(identity: CameraIdentity) -> Self {
        Self::Camera(Arc::new(identity))
    }

    pub fn unbound() -> Self {
        Self::Unbound(Arc::new(()))
    }

    pub fn captured(&self) -> bool {
        !matches!(self, Self::Unbound(_))
    }

    pub fn proxy(proxy: &crate::scan::CameraPixels) -> Self {
        proxy
            .identity()
            .cloned()
            .map(Self::camera)
            .unwrap_or_else(|| Self::Proxy(Arc::downgrade(&proxy.pixels)))
    }
}
