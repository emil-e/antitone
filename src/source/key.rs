//! Identity shared by source acquisition and retained pixel storage.
use super::FileKey;
use crate::{
    project::SourceDevelopment,
    scan::{CameraPixels, DevelopedSource},
};
use std::path::{Path, PathBuf};

/// Capture before development; grade/crop/correction/target/priority and SourceId are excluded.
#[derive(Clone, Debug)]
pub struct SourceKey {
    pub(super) locator: PathBuf,
    pub(super) file: FileKey,
    pub(super) source: SourceDevelopment,
}

impl SourceKey {
    /// Capture identity when opening a source. Reuse the key for acquisitions in that session.
    pub fn new(path: &Path, source: &SourceDevelopment) -> Self {
        let locator = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().map_or_else(|_| path.to_path_buf(), |cwd| cwd.join(path))
        };
        Self::for_load(&locator, FileKey::new(&locator), source)
    }

    pub(crate) fn for_load(path: &Path, file: FileKey, source: &SourceDevelopment) -> Self {
        Self {
            locator: path.to_path_buf(),
            file,
            source: source.for_runtime(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.locator
    }

    #[cfg(feature = "editor")]
    pub(crate) fn file(&self) -> &FileKey {
        &self.file
    }

    pub fn development(&self) -> &SourceDevelopment {
        &self.source
    }

    pub fn capture(path: &Path, source: &SourceDevelopment) -> Option<Self> {
        let locator = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().ok()?.join(path)
        };
        Self::from_revision(&locator, FileKey::new(&locator), source)
    }

    pub(crate) fn from_revision(
        path: &Path,
        file: FileKey,
        source: &SourceDevelopment,
    ) -> Option<Self> {
        file.revision.as_ref()?;
        Some(Self::for_load(path, file, source))
    }

    pub(crate) fn matches(&self, source: &DevelopedSource) -> bool {
        let Ok(image) = source.camera_image() else {
            return false;
        };
        let area = source.area();
        let actual = SourceDevelopment {
            decode: source.metadata().development(source.white_balance()),
            dimensions: [image.width, image.height],
            active_area: [area.x, area.y, area.width, area.height],
            camera_model: source.camera_model().to_owned(),
            camera_to_ap1: source.camera_to_ap1().to_cols_array_2d(),
        };
        self.source.same_processing(&actual)
            && self.source.decode.backend_version == actual.decode.backend_version
    }

    pub(super) fn matches_proxy(&self, proxy: &CameraPixels) -> bool {
        proxy.width > 0
            && proxy.height > 0
            && proxy.width <= CameraPixels::MAX_EDGE
            && proxy.height <= CameraPixels::MAX_EDGE
            && proxy.pixels.len() == proxy.width as usize * proxy.height as usize
            && proxy.matrix.to_cols_array_2d() == self.source.camera_to_ap1
    }
}

impl PartialEq for SourceKey {
    fn eq(&self, other: &Self) -> bool {
        self.file == other.file
            && self.source.same_processing(&other.source)
            && self.source.decode.backend_version == other.source.decode.backend_version
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquisition_targets_current_libraw_while_cache_identity_distinguishes_pixel_producers() {
        let file = crate::raw::sony_fixture::source(
            crate::raw::sony_fixture::Order::Little,
            vec![],
            false,
            false,
        );
        let raw = crate::raw::RawFile::open(file.path()).unwrap();
        let mut recorded = SourceDevelopment::from_metadata(&raw.metadata, [1.; 4]).unwrap();
        assert!(!recorded.decode.uses_sdk());
        // The same LibRaw release without our development revision is a different producer.
        recorded.decode.backend_version = crate::raw::version();
        let current = SourceKey::capture(file.path(), &recorded).unwrap();
        assert_eq!(
            current.development().decode.backend_version,
            crate::raw::development_version()
        );
        let mut previous_producer = current.clone();
        previous_producer.source.decode.backend_version = recorded.decode.backend_version;
        assert_ne!(
            previous_producer, current,
            "pixels produced by different runtimes must not share a cache identity"
        );
    }
}
