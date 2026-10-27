//! Identity shared by source acquisition and retained pixel storage.
use super::FileKey;
use crate::{
    project::SourceDevelopment,
    scan::{CameraProxy, DevelopedSource},
};
use std::path::{Path, PathBuf};

/// Capture before development; grade/crop/correction/target/priority and SourceId are excluded.
#[derive(Clone, Debug)]
pub struct SourceKey {
    locator: PathBuf,
    file: FileKey,
    source: SourceDevelopment,
}

#[cfg(test)]
impl SourceKey {
    pub fn capture_with_development(
        path: &Path,
        source: &SourceDevelopment,
        _development: &crate::project::Development,
    ) -> Option<Self> {
        Self::capture(path, source)
    }

    pub(crate) fn from_revision_with_development(
        path: &Path,
        file: FileKey,
        source: &SourceDevelopment,
        _development: &crate::project::Development,
    ) -> Option<Self> {
        Self::from_revision(path, file, source)
    }
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
            source: source.clone(),
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
        Some(Self {
            locator: path.to_path_buf(),
            file,
            source: source.clone(),
        })
    }

    pub(crate) fn matches(&self, source: &DevelopedSource) -> bool {
        let Ok(image) = source.camera_image() else {
            return false;
        };
        let area = source.area();
        let Ok(lens_correction) = source.metadata().lens_identity() else {
            return false;
        };
        self.source
            == SourceDevelopment {
                decode: source.metadata().development(source.fixed_wb()),
                lens_correction,
                dng_development: source.metadata().dng_identity(),
                dimensions: [image.width, image.height],
                active_area: [area.x, area.y, area.width, area.height],
                camera_model: source.camera_model().to_owned(),
                camera_to_ap1: source.camera_to_ap1().to_cols_array_2d(),
                input_floor: source.input_floor(),
            }
    }

    pub(super) fn matches_proxy(&self, proxy: &CameraProxy) -> bool {
        proxy.width > 0
            && proxy.height > 0
            && proxy.width <= CameraProxy::MAX_EDGE
            && proxy.height <= CameraProxy::MAX_EDGE
            && proxy.pixels.len() == proxy.width as usize * proxy.height as usize
            && proxy.matrix.to_cols_array_2d() == self.source.camera_to_ap1
    }
}

impl PartialEq for SourceKey {
    fn eq(&self, other: &Self) -> bool {
        self.file == other.file && self.source == other.source
    }
}
