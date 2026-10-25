use serde::{Deserialize, Serialize};
#[cfg(feature = "raw")]
use std::path::PathBuf;
use std::{path::Path, time::SystemTime};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileRevision {
    pub(crate) bytes: u64,
    pub(crate) modified: SystemTime,
    #[serde(default)]
    pub(crate) device: u64,
    #[serde(default)]
    pub(crate) inode: u64,
    #[serde(default)]
    pub(crate) changed: (i64, i64),
}

impl FileRevision {
    pub fn read(path: &Path) -> std::io::Result<Self> {
        let metadata = std::fs::metadata(path)?;
        let identity = crate::sys::file_identity(path)?;
        Ok(Self {
            bytes: metadata.len(),
            modified: metadata.modified()?,
            device: identity.device,
            inode: identity.inode,
            changed: identity.changed,
        })
    }
}

/// Disposable freshness, not photograph identity or permission to accept a grade.
#[cfg(feature = "raw")]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileKey {
    pub(crate) path: PathBuf,
    pub(crate) revision: Option<FileRevision>,
}

#[cfg(feature = "raw")]
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
