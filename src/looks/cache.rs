//! Disposable generated-treatment cache. Saved projects own their accepted samples.
use super::{LookAsset, LookOrigin, failure, material::Selection, require};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
#[cfg(test)]
mod photochemical_tests;
const MAX_FILE_BYTES: u64 = 48 * 1024 * 1024;
const BUDGET: u64 = 64 * 1024 * 1024;
pub struct Cache {
    root: PathBuf,
}

impl Cache {
    pub fn at(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn default_location() -> Option<Self> {
        crate::sys::cache_dir("antitone").map(|root| Self::at(root.join("looks-v1")))
    }

    fn path(&self, origin: &LookOrigin, size: u32) -> Result<PathBuf, crate::Error> {
        let key = serde_json::to_vec(&(super::CONTRACT, origin, size)).map_err(failure)?;
        Ok(self
            .root
            .join(format!("{}.json", blake3::hash(&key).to_hex())))
    }

    pub fn load_photochemical(
        &self,
        selection: impl Into<Selection>,
        size: u32,
    ) -> Result<Option<LookAsset>, crate::Error> {
        let selection = selection.into();
        self.load_origin(&selection.origin(), &selection.name(), size)
    }

    fn load_origin(
        &self,
        origin: &LookOrigin,
        name: &str,
        size: u32,
    ) -> Result<Option<LookAsset>, crate::Error> {
        let path = self.path(origin, size)?;
        let file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(failure(e)),
        };
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(failure)?;
        require(
            bytes.len() as u64 <= MAX_FILE_BYTES,
            "cached LUT exceeds the file budget",
        )?;
        let asset: LookAsset = serde_json::from_slice(&bytes).map_err(failure)?;
        require(
            asset.origin() == origin && asset.lut().size() == size,
            "cached LUT source/specification mismatch",
        )?;
        // Cache labels follow the current catalog; embedded project assets do not.
        let asset = LookAsset::new(name.into(), asset.origin().clone(), asset.lut().clone())?;
        // Optional recency metadata; failure does not invalidate verified samples.
        if let Ok(file) = fs::OpenOptions::new().write(true).open(&path) {
            let _ = file.set_times(fs::FileTimes::new().set_modified(std::time::SystemTime::now()));
        }
        Ok(Some(asset))
    }

    pub fn store_photochemical(
        &self,
        selection: impl Into<Selection>,
        asset: &LookAsset,
    ) -> Result<(), crate::Error> {
        let selection = selection.into();
        self.store_origin(&selection.origin(), asset)
    }

    fn store_origin(&self, origin: &LookOrigin, asset: &LookAsset) -> Result<(), crate::Error> {
        require(
            asset.origin() == origin,
            "cache source differs from generated look",
        )?;
        let path = self.path(origin, asset.lut().size())?;
        let bytes = serde_json::to_vec(asset).map_err(failure)?;
        require(
            bytes.len() as u64 <= MAX_FILE_BYTES,
            "generated LUT exceeds the file budget",
        )?;
        fs::create_dir_all(&self.root).map_err(failure)?;
        let mut temporary = tempfile::NamedTempFile::new_in(&self.root).map_err(failure)?;
        temporary.write_all(&bytes).map_err(failure)?;
        temporary.as_file().sync_all().map_err(failure)?;
        temporary.persist(&path).map_err(failure)?;
        self.prune(&path)
    }

    fn prune(&self, keep: &Path) -> Result<(), crate::Error> {
        let mut files = Vec::new();
        let mut total = 0u64;
        for entry in fs::read_dir(&self.root).map_err(failure)? {
            let entry = entry.map_err(failure)?;
            let path = entry.path();
            // Never remove unrelated files or follow symlinks in the cache namespace.
            let Some(stem) = path.file_stem().and_then(|n| n.to_str()) else {
                continue;
            };
            if path.extension().and_then(|s| s.to_str()) != Some("json")
                || stem.len() != 64
                || !stem.bytes().all(|b| b.is_ascii_hexdigit())
                || entry.file_type().map_err(failure)?.is_symlink()
            {
                continue;
            }
            let metadata = entry.metadata().map_err(failure)?;
            if !metadata.is_file() {
                continue;
            }
            total = total.saturating_add(metadata.len());
            files.push((metadata.modified().ok(), path, metadata.len()));
        }
        files.sort_by_key(|a| a.0);
        for (_, path, bytes) in files {
            if total <= BUDGET {
                break;
            }
            if path != keep {
                fs::remove_file(path).map_err(failure)?;
                total = total.saturating_sub(bytes);
            }
        }
        Ok(())
    }
}
