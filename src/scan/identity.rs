use super::*;
use std::sync::Weak;

/// Captured developed-camera content, independent of interpretation and publication.
/// Retaining this value keeps provenance, not a native pixel lease.
#[derive(Clone)]
pub struct CameraIdentity {
    native: Weak<FrozenRaw>,
    capture: crate::source::FileKey,
    white_balance: [f64; 4],
    lens_correction: bool,
    dng_interpretation: Option<String>,
    area: Rect,
    matrix: DMat3,
    scale: f64,
    floor: f64,
}

impl PartialEq for CameraIdentity {
    fn eq(&self, other: &Self) -> bool {
        let same_capture = if self.capture.revision.is_some() && other.capture.revision.is_some() {
            self.capture == other.capture
        } else {
            self.native.ptr_eq(&other.native)
        };
        same_capture
            && self.white_balance == other.white_balance
            && self.lens_correction == other.lens_correction
            && self.dng_interpretation == other.dng_interpretation
            && self.area == other.area
            && self.matrix == other.matrix
            && self.scale == other.scale
            && self.floor == other.floor
    }
}

impl std::hash::Hash for CameraIdentity {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // A native allocation carries one immutable file key, including revision availability.
        // Reloads can compare equal only when both carry the same versioned key.
        match self.capture.revision {
            Some(_) => self.capture.hash(state),
            None => self.native.as_ptr().hash(state),
        }
        self.area.hash(state);
        self.lens_correction.hash(state);
        self.dng_interpretation.hash(state);
        for value in self
            .white_balance
            .into_iter()
            .chain([self.scale, self.floor])
        {
            crate::float_hash::f64(value, state);
        }
    }
}

impl CameraIdentity {
    pub(super) fn heap_bytes(&self) -> u64 {
        self.capture.path.capacity() as u64
            + self
                .dng_interpretation
                .as_ref()
                .map_or(0, |v| v.capacity() as u64)
    }
}

impl DevelopedSource {
    pub fn identity(&self) -> CameraIdentity {
        let metadata = self.metadata();
        CameraIdentity {
            native: Arc::downgrade(&self.raw),
            capture: self.file.clone(),
            white_balance: self.white_balance,
            lens_correction: metadata.lens_correction_enabled(),
            dng_interpretation: metadata.dng_identity(),
            area: self.area,
            matrix: self.camera_to_ap1,
            scale: self.source_scale,
            floor: self.precision_floor,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fingerprint(identity: &CameraIdentity) -> u64 {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let mut state = DefaultHasher::new();
        identity.hash(&mut state);
        state.finish()
    }

    #[test]
    fn camera_identity_is_shared_with_proxies_and_reloads_without_retaining_native_pixels() {
        let file = crate::raw::fixture::bayer_dng();
        let scan = Scan::open(file.path(), Some([1.; 4])).unwrap();
        let identity = scan.source().identity();
        let native = Arc::downgrade(&scan.source.raw);
        let proxy = CameraProxy::from_scan(&scan).unwrap();
        assert!(proxy.identity() == Some(&identity));
        let reloaded = Scan::open(file.path(), Some([1.; 4])).unwrap();
        let reloaded_identity = reloaded.source().identity();
        assert!(identity == reloaded_identity);
        assert_eq!(fingerprint(&identity), fingerprint(&reloaded_identity));
        let other = crate::raw::fixture::bayer_dng();
        let other = Scan::open(other.path(), Some([1.; 4])).unwrap();
        assert!(
            identity != other.source().identity(),
            "same development is not the same capture"
        );
        drop(scan);
        assert!(
            native.upgrade().is_none(),
            "source identity and a proxy do not keep original storage alive"
        );
        assert!(proxy.identity() == Some(&identity));
    }
}
