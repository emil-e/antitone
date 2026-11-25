//! Sampling policy and acquisition needs derived from complete photographic intent.

#[cfg(feature = "gpu")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Quality {
    Originals,
    CameraProxy,
}

#[cfg(feature = "gpu")]
impl Quality {
    pub fn includes_repairs(self) -> bool {
        self == Self::Originals
    }
}
