//! Sampling policy and acquisition needs derived from complete photographic intent.

#[cfg(feature = "gpu")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Quality {
    Originals,
    #[cfg(feature = "editor")]
    PhysicalView,
    CameraProxy,
    #[cfg(feature = "editor")]
    Measurement,
}

#[cfg(feature = "gpu")]
impl Quality {
    pub fn includes_repairs(self) -> bool {
        match self {
            Self::Originals => true,
            #[cfg(feature = "editor")]
            Self::PhysicalView => true,
            _ => false,
        }
    }
}
