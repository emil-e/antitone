//! Disposable built-in bake cache. Saved projects never depend on this directory.
use super::{LookAsset, builtin::Builtin, failure, require};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
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
        std::env::var_os("HOME")
            .map(|home| Self::at(PathBuf::from(home).join(".cache/antitone/looks-v1")))
    }

    fn path(&self, builtin: Builtin, size: u32) -> Result<PathBuf, crate::Error> {
        let key =
            serde_json::to_vec(&(super::CONTRACT, builtin.origin(), size)).map_err(failure)?;
        Ok(self
            .root
            .join(format!("{}.json", blake3::hash(&key).to_hex())))
    }

    pub fn load(&self, builtin: Builtin, size: u32) -> Result<Option<LookAsset>, crate::Error> {
        let path = self.path(builtin, size)?;
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
            asset.origin() == &builtin.origin() && asset.lut().size() == size,
            "cached LUT source/specification mismatch",
        )?;
        // Cache labels follow the current catalog; embedded project assets do not.
        let asset = LookAsset::new(
            builtin.name().into(),
            asset.origin().clone(),
            asset.lut().clone(),
        )?;
        // Optional recency metadata; failure does not invalidate verified samples.
        if let Ok(file) = fs::OpenOptions::new().write(true).open(&path) {
            let _ = file.set_times(fs::FileTimes::new().set_modified(std::time::SystemTime::now()));
        }
        Ok(Some(asset))
    }

    pub fn store(&self, builtin: Builtin, asset: &LookAsset) -> Result<(), crate::Error> {
        require(
            asset.origin() == &builtin.origin(),
            "cache source differs from generated look",
        )?;
        let path = self.path(builtin, asset.lut().size())?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn cached_assets_are_validated_and_projects_need_no_cache() {
        let directory = tempfile::tempdir().unwrap();
        let cache = Cache::at(directory.path().into());
        let builtin = Builtin::PortraEndura;
        assert!(cache.load(builtin, 3).unwrap().is_none());
        let asset = LookAsset::new(
            "source".into(),
            builtin.origin(),
            Arc::new(super::super::tests::identity(3)),
        )
        .unwrap();
        cache.store(builtin, &asset).unwrap();
        let cached = cache.load(builtin, 3).unwrap().unwrap();
        assert_eq!(cached.name(), builtin.name());
        assert_eq!(cached.origin(), asset.origin());
        assert_eq!(cached.lut(), asset.lut());
        let restored: LookAsset =
            serde_json::from_slice(&serde_json::to_vec(&asset).unwrap()).unwrap();
        assert_eq!(restored, asset);
        assert_eq!(restored.name(), "source");
        assert!(cache.load(Builtin::Kodak2383, 3).unwrap().is_none());
        fs::write(cache.path(builtin, 3).unwrap(), b"corrupt").unwrap();
        assert!(cache.load(builtin, 3).is_err());
        assert!(asset.lut().apply(glam::DVec3::splat(0.18)).is_ok());
    }
}
