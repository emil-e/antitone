use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileRevision {
    pub(crate) bytes: u64,
    pub(crate) modified: SystemTime,
    #[cfg(unix)]
    pub(crate) device: u64,
    #[cfg(unix)]
    pub(crate) inode: u64,
    #[cfg(unix)]
    pub(crate) changed: (i64, i64),
}
impl FileRevision {
    fn read(path: &Path) -> std::io::Result<Self> {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::metadata(path)?;
        Ok(Self {
            bytes: metadata.len(),
            modified: metadata.modified()?,
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
            #[cfg(unix)]
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        })
    }
}

/// Disposable freshness, not photograph identity or permission to accept a grade.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileKey {
    pub(crate) path: PathBuf,
    pub(crate) revision: Option<FileRevision>,
}
impl FileKey {
    pub(crate) fn new(path: &Path) -> Self {
        Self {
            path: path.canonicalize().unwrap_or_else(|_| path.to_path_buf()),
            // Unavailable stats disable reuse; authoritative loading reports source errors.
            revision: FileRevision::read(path).ok(),
        }
    }
    #[cfg(all(feature = "raw", feature = "project"))]
    pub(crate) fn current_at(&self, locator: &Path) -> bool {
        self.revision.is_some() && Self::new(locator) == *self
    }
}
