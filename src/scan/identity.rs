use super::*;
use std::sync::Weak;

/// Captured developed-camera content, independent of interpretation and publication.
/// Retaining this value keeps provenance, not a native pixel lease.
#[derive(Clone)]
pub struct CameraIdentity {
    native: Weak<FrozenRaw>,
    capture: crate::source::FileKey,
    fixed_wb: [f64; 4],
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
            && self.fixed_wb == other.fixed_wb
            && self.lens_correction == other.lens_correction
            && self.dng_interpretation == other.dng_interpretation
            && self.area == other.area
            && self.matrix == other.matrix
            && self.scale == other.scale
            && self.floor == other.floor
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
            fixed_wb: self.fixed_wb,
            lens_correction: metadata.lens_correction_enabled(),
            dng_interpretation: metadata.dng_identity(),
            area: self.area,
            matrix: self.camera_to_ap1,
            scale: self.source_scale,
            floor: self.input_floor,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn camera_identity_is_shared_with_proxies_and_reloads_without_retaining_native_pixels() {
        let file = crate::raw::fixture::bayer_dng();
        let scan = Scan::open(file.path(), Some([1.; 4])).unwrap();
        let identity = scan.source().identity();
        let native = Arc::downgrade(&scan.source.raw);
        let proxy = CameraProxy::from_scan(&scan).unwrap();
        assert!(proxy.identity() == Some(&identity));
        let reloaded = Scan::open(file.path(), Some([1.; 4])).unwrap();
        assert!(identity == reloaded.source().identity());
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
