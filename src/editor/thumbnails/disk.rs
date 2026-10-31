use super::{FileKey, Proxy, SOURCE_SIZE, SourceKey};
#[cfg(test)]
use crate::project::Development;
use crate::project::SourceDevelopment;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read, Write},
    path::PathBuf,
    sync::Arc,
};

const MAGIC: &[u8; 8] = b"ANTTHM05";
const LIMIT: u64 = 128 << 20;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    file: FileKey,
    source: SourceDevelopment,
    width: u32,
    height: u32,
    checksum: String,
}

pub(super) struct DiskCache {
    root: PathBuf,
    budget: u64,
}

impl DiskCache {
    #[cfg(test)]
    pub(super) fn at(root: PathBuf) -> Self {
        Self {
            root,
            budget: LIMIT,
        }
    }

    pub fn default_location() -> Option<Self> {
        crate::sys::cache_dir("antitone").map(|root| Self {
            root: root.join("thumbnails-v5"),
            budget: LIMIT,
        })
    }

    fn path(&self, key: &SourceKey) -> io::Result<PathBuf> {
        if key.file.revision.is_none() {
            return Err(invalid("source file stats unavailable for thumbnail reuse"));
        }
        // File stats are disposable cache freshness, not persisted photograph identity.
        let identity = serde_json::to_vec(&(
            5,
            crate::exposure_metadata::VERSION,
            SOURCE_SIZE,
            Proxy::SAMPLING_VERSION,
            &key.file,
            &key.source,
        ))?;
        Ok(self
            .root
            .join(format!("{}.camera32", blake3::hash(&identity).to_hex())))
    }

    pub fn load(&self, key: &SourceKey) -> io::Result<Option<Arc<Proxy>>> {
        let path = self.path(key)?;
        let mut file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let mut prefix = [0; 12];
        file.read_exact(&mut prefix)?;
        if &prefix[..8] != MAGIC {
            return Err(invalid("thumbnail cache version"));
        }
        let length = u32::from_le_bytes(prefix[8..].try_into().unwrap()) as usize;
        if length > 16 * 1024 {
            return Err(invalid("thumbnail cache header size"));
        }
        let mut bytes = vec![0; length];
        file.read_exact(&mut bytes)?;
        let header: Header = serde_json::from_slice(&bytes)?;
        let [_, _, width, height] = key.source.active_area;
        let scale = (SOURCE_SIZE as f64 / width.max(height) as f64).min(1.);
        let dimensions = [
            (width as f64 * scale).round().max(1.) as u32,
            (height as f64 * scale).round().max(1.) as u32,
        ];
        if header.file != key.file
            || header.source != key.source
            || [header.width, header.height] != dimensions
            || dimensions.iter().any(|n| *n == 0 || *n > SOURCE_SIZE)
        {
            return Err(invalid("thumbnail cache identity/dimensions"));
        }
        let mut pixels = vec![[0f32; 4]; header.width as usize * header.height as usize];
        let bytes = bytemuck::cast_slice_mut(&mut pixels);
        file.read_exact(bytes)?;
        let mut tail = [0; 1];
        if file.read(&mut tail)? != 0 || checksum(bytes)? != header.checksum {
            return Err(invalid("thumbnail cache pixel checksum/length"));
        }
        if pixels.iter().flatten().any(|v| {
            !v.is_finite() || !(0. ..=crate::processing::MAX_CAMERA_SOURCE_CODE).contains(v)
        }) {
            return Err(invalid("thumbnail cache continuous sample range"));
        }

        // Disposable LRU access time; failure here doesn't invalidate verified cached pixels.
        let _ = file.set_times(fs::FileTimes::new().set_modified(std::time::SystemTime::now()));
        Ok(Some(Arc::new(Proxy {
            identity: None,
            width: header.width,
            height: header.height,
            pixels: std::sync::Arc::new(pixels),
            matrix: glam::DMat3::from_cols_array_2d(&header.source.camera_to_ap1),
        })))
    }

    pub fn store_if_missing(&self, key: &SourceKey, proxy: &Proxy) -> io::Result<()> {
        if !self.path(key)?.exists() {
            self.store(key, proxy)?;
        }
        Ok(())
    }

    pub fn store(&self, key: &SourceKey, proxy: &Proxy) -> io::Result<()> {
        fs::create_dir_all(&self.root)?;
        let bytes = bytemuck::cast_slice(&proxy.pixels);

        let header = serde_json::to_vec(&Header {
            file: key.file.clone(),
            source: key.source.clone(),
            width: proxy.width,
            height: proxy.height,
            checksum: checksum(bytes)?,
        })?;
        let mut file = tempfile::NamedTempFile::new_in(&self.root)?;
        file.write_all(MAGIC)?;
        file.write_all(&(header.len() as u32).to_le_bytes())?;
        file.write_all(&header)?;
        file.write_all(bytes)?;
        file.persist(self.path(key)?).map_err(|e| e.error)?;
        self.prune()
    }

    fn prune(&self) -> io::Result<()> {
        let mut entries = Vec::new();
        let mut total = 0;
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if entry.path().extension().is_some_and(|e| e == "camera32") {
                let metadata = entry.metadata()?;
                total += metadata.len();
                entries.push((metadata.modified()?, metadata.len(), entry.path()));
            }
        }
        entries.sort_by_key(|entry| entry.0);
        for (_, size, path) in entries {
            if total <= self.budget {
                break;
            }
            match fs::remove_file(path) {
                Ok(()) => total -= size,
                Err(error) if error.kind() == io::ErrorKind::NotFound => total -= size,
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

fn checksum(pixels: &[u8]) -> io::Result<String> {
    let mut digest = blake3::Hasher::new();
    digest.update(pixels);

    Ok(digest.finalize().to_hex().to_string())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (SourceKey, Proxy) {
        let key = SourceKey {
            id: crate::project::SourceId(1),
            file: FileKey {
                path: "/fixture.ARW".into(),
                revision: Some(super::super::FileRevision {
                    bytes: 1,
                    modified: std::time::UNIX_EPOCH,
                    device: 1,
                    inode: 1,
                    changed: (1, 0),
                }),
            },
            source: SourceDevelopment {
                decode: Development::new("test".into(), [1.; 4]),
                lens_correction: Some("00".repeat(32)),
                dng_development: None,
                dimensions: [2, 1],
                active_area: [0, 0, 2, 1],
                camera_model: "test".into(),
                camera_to_ap1: glam::DMat3::IDENTITY.to_cols_array_2d(),
                input_floor: crate::processing::CAMERA_INPUT_FLOOR,
            },
        };
        let proxy = Proxy {
            identity: None,
            width: 2,
            height: 1,
            pixels: vec![
                [1.25, 2.5, 3.75, 0.],
                [
                    65536.,
                    crate::processing::MAX_CAMERA_SOURCE_CODE,
                    12345.125,
                    0.,
                ],
            ]
            .into(),
            matrix: glam::DMat3::IDENTITY,
        };
        (key, proxy)
    }

    #[test]
    fn exact_pixels_survive_restart_and_source_ids_do_not_affect_disk_identity() {
        let directory = tempfile::tempdir().unwrap();
        let cache = DiskCache {
            root: directory.path().into(),
            budget: LIMIT,
        };
        let (mut key, proxy) = fixture();
        cache.store(&key, &proxy).unwrap();
        key.id = crate::project::SourceId(999);
        let cache = DiskCache {
            root: directory.path().into(),
            budget: LIMIT,
        };
        assert_eq!(cache.load(&key).unwrap().unwrap().pixels, proxy.pixels);
        key.source.decode.fixed_wb[0] = 2.;
        assert!(cache.load(&key).unwrap().is_none());
    }

    #[test]
    fn decoder_version_changes_are_cache_misses_not_file_admission_errors() {
        let directory = tempfile::tempdir().unwrap();
        let cache = DiskCache::at(directory.path().into());
        let (key, proxy) = fixture();
        cache.store(&key, &proxy).unwrap();
        let mut current = key.clone();
        current.source.decode.backend_version.push_str("-new");
        assert!(key.source.same_processing(&current.source));
        assert!(cache.load(&current).unwrap().is_none());
        assert_eq!(cache.load(&key).unwrap().unwrap().pixels, proxy.pixels);
    }

    #[test]
    fn point_sampled_cache_entries_do_not_supply_the_filtered_proxy_contract() {
        let directory = tempfile::tempdir().unwrap();
        let cache = DiskCache::at(directory.path().into());
        let (key, proxy) = fixture();
        cache.store(&key, &proxy).unwrap();
        // The preceding derivation identity had no camera-proxy sampling revision.
        let legacy = serde_json::to_vec(&(3, SOURCE_SIZE, &key.file, &key.source)).unwrap();
        let legacy_path = directory
            .path()
            .join(format!("{}.camera32", blake3::hash(&legacy).to_hex()));
        fs::rename(cache.path(&key).unwrap(), legacy_path).unwrap();
        assert!(cache.load(&key).unwrap().is_none());
    }

    #[test]
    fn changed_stats_locators_and_unavailable_stats_do_not_reuse_disk_pixels() {
        let directory = tempfile::tempdir().unwrap();
        let cache = DiskCache::at(directory.path().into());
        let (key, proxy) = fixture();
        cache.store(&key, &proxy).unwrap();
        let mut changed = key.clone();
        changed.file.revision.as_mut().unwrap().bytes += 1;
        assert!(cache.load(&changed).unwrap().is_none());
        changed = key.clone();
        changed.file.path = "/other-card/fixture.ARW".into();
        assert!(cache.load(&changed).unwrap().is_none());
        changed = key.clone();
        changed.source.lens_correction = Some("11".repeat(32));
        assert!(cache.load(&changed).unwrap().is_none());
        changed.file.revision = None;
        assert!(cache.load(&changed).is_err());
        assert!(cache.store(&changed, &proxy).is_err());
        assert_eq!(cache.load(&key).unwrap().unwrap().pixels, proxy.pixels);
    }

    #[test]
    fn valid_checksums_do_not_admit_nonfinite_negative_or_excessive_camera_values() {
        let directory = tempfile::tempdir().unwrap();
        let cache = DiskCache::at(directory.path().into());
        for value in [
            f32::NAN,
            -1.,
            crate::processing::MAX_CAMERA_SOURCE_CODE + 1.,
        ] {
            let (key, mut proxy) = fixture();
            std::sync::Arc::make_mut(&mut proxy.pixels)[0][0] = value;
            cache.store(&key, &proxy).unwrap();
            assert!(cache.load(&key).is_err());
        }
    }

    #[test]
    fn corrupt_truncated_and_wrong_identity_entries_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let cache = DiskCache {
            root: directory.path().into(),
            budget: LIMIT,
        };
        let (key, proxy) = fixture();
        for truncate in [false, true] {
            cache.store(&key, &proxy).unwrap();
            let path = cache.path(&key).unwrap();
            let mut bytes = fs::read(&path).unwrap();
            if truncate {
                bytes.pop();
            } else {
                *bytes.last_mut().unwrap() ^= 1;
            }
            fs::write(&path, bytes).unwrap();
            assert!(cache.load(&key).is_err());
        }
        cache.store(&key, &proxy).unwrap();
        let mut other = key.clone();
        other.source.camera_to_ap1[0][0] = 2.;
        fs::copy(cache.path(&key).unwrap(), cache.path(&other).unwrap()).unwrap();
        assert!(cache.load(&other).is_err());
    }

    #[test]

    fn disk_pruning_bounds_storage_and_unavailable_cache_is_an_io_error() {
        let directory = tempfile::tempdir().unwrap();
        let cache = DiskCache {
            root: directory.path().into(),
            budget: 1,
        };
        let (key, proxy) = fixture();
        cache.store(&key, &proxy).unwrap();
        assert!(cache.load(&key).unwrap().is_none());
        let file = directory.path().join("file");
        fs::write(&file, []).unwrap();
        assert!(
            DiskCache {
                root: file,
                budget: LIMIT
            }
            .store(&key, &proxy)
            .is_err()
        );
    }
}
