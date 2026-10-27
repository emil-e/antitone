#[cfg(feature = "project")]
use serde::{Deserialize, Serialize};
#[cfg(feature = "raw")]
use std::path::PathBuf;
use std::{path::Path, time::SystemTime};

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "project", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "project", serde(deny_unknown_fields))]
pub struct FileRevision {
    pub(crate) bytes: u64,
    pub(crate) modified: SystemTime,
    #[cfg_attr(feature = "project", serde(default))]
    pub(crate) device: u64,
    #[cfg_attr(feature = "project", serde(default))]
    pub(crate) inode: u64,
    #[cfg_attr(feature = "project", serde(default))]
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
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "project", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "project", serde(deny_unknown_fields))]
pub(crate) struct FileKey {
    pub(crate) path: PathBuf,
    pub(crate) revision: Option<FileRevision>,
}

#[cfg(feature = "raw")]
impl FileKey {
    pub(crate) fn new(path: &Path) -> Self {
        Self {
            path: path.canonicalize().unwrap_or_else(|_| path.to_path_buf()),
            // Missing metadata leaves persisted cache identity unavailable.
            revision: FileRevision::read(path).ok(),
        }
    }

    #[cfg(feature = "editor")]
    pub(crate) fn unversioned(path: &Path) -> Self {
        Self {
            path: path.to_owned(),
            revision: None,
        }
    }
}
