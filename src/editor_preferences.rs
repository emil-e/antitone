use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::Write, path::PathBuf};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Preferences {
    version: u32,
    pub(super) sections: BTreeMap<String, bool>,
    pub(super) scopes_visible: bool,
    pub(super) scope_height_ratio: f32,
    pub(super) filmstrip_height: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) cache_limits: Option<super::cache_limits::CacheLimits>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) export_folder: Option<PathBuf>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 1,
            sections: BTreeMap::new(),
            scopes_visible: true,
            scope_height_ratio: 0.85,
            filmstrip_height: 140.,
            cache_limits: None,
            export_folder: None,
        }
    }
}

impl Preferences {
    fn validate(&self) -> Result<()> {
        if self.version != 1 {
            return Err(format!("unsupported UI preferences version {}", self.version).into());
        }
        if self.cache_limits.is_some_and(|limits| !limits.valid()) {
            return Err("invalid cache limits: use whole MiB within the byte counter range".into());
        }
        if !self.scope_height_ratio.is_finite()
            || self.scope_height_ratio <= 0.
            || !self.filmstrip_height.is_finite()
            || !(52.0..=240.0).contains(&self.filmstrip_height)
        {
            return Err("invalid UI preference dimensions".into());
        }
        Ok(())
    }
}

pub(super) struct Store {
    path: PathBuf,
    saved: Option<Preferences>,
}

impl Store {
    pub(super) fn open_default() -> Result<(Self, Preferences)> {
        let directory = crate::sys::config_dir("Antitone")
            .ok_or("user configuration directory is unavailable for UI preferences")?;
        Self::open(directory.join("ui-preferences.json"))
    }

    pub(super) fn open(path: PathBuf) -> Result<(Self, Preferences)> {
        let saved = match std::fs::read(&path) {
            Ok(bytes) => {
                let preferences: Preferences = serde_json::from_slice(&bytes)?;
                preferences.validate()?;
                Some(preferences)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let preferences = saved.clone().unwrap_or_default();
        Ok((Self { path, saved }, preferences))
    }

    pub(super) fn save(&mut self, preferences: &Preferences) -> Result<()> {
        preferences.validate()?;
        if self.saved.as_ref() == Some(preferences) {
            return Ok(());
        }
        let parent = self
            .path
            .parent()
            .ok_or("UI preferences have no parent directory")?;
        std::fs::create_dir_all(parent)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(&serde_json::to_vec_pretty(preferences)?)?;
        temporary.as_file().sync_all()?;
        temporary.persist(&self.path)?;
        self.saved = Some(preferences.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_preferences_and_partial_objects_use_first_run_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        let (_, preferences) = Store::open(path.clone()).unwrap();
        assert_eq!(preferences, Preferences::default());
        std::fs::write(&path, r#"{"scopes_visible":false,"future_hint":true}"#).unwrap();
        let (_, preferences) = Store::open(path).unwrap();
        assert!(!preferences.scopes_visible);
        assert_eq!(preferences.scope_height_ratio, 0.85);
        assert_eq!(preferences.filmstrip_height, 140.);
    }

    #[test]
    fn preferences_round_trip_atomically_without_project_or_view_state() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested/preferences.json");
        let (mut store, mut preferences) = Store::open(path.clone()).unwrap();
        preferences.sections.insert("roll-exposure".into(), true);
        preferences.sections.insert("photo-exposure".into(), false);
        preferences.scopes_visible = false;
        preferences.scope_height_ratio = 1.2345;
        preferences.filmstrip_height = 193.5;
        store.save(&preferences).unwrap();
        let (mut reopened, restored) = Store::open(path.clone()).unwrap();
        assert_eq!(restored, preferences);
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        reopened.save(&restored).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            before
        );
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 5);
        assert_eq!(
            std::fs::read_dir(directory.path().join("nested"))
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn chosen_cache_limits_persist_including_zero_and_survive_other_hardware_defaults() {
        use super::super::cache_limits::{CacheLimits, Hardware, MIB};
        use crate::sys::GpuMemory;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        std::fs::write(&path, r#"{"scopes_visible":false}"#).unwrap();
        let (mut store, mut preferences) = Store::open(path.clone()).unwrap();
        assert!(preferences.cache_limits.is_none());
        let first = Hardware {
            ram_bytes: Some(16 << 30),
            gpu: GpuMemory::Unified,
        }
        .defaults();
        preferences.cache_limits.get_or_insert(first);
        store.save(&preferences).unwrap();
        let (mut store, mut restored) = Store::open(path.clone()).unwrap();
        let other = Hardware {
            ram_bytes: Some(64 << 30),
            gpu: GpuMemory::Unknown,
        }
        .defaults();
        restored.cache_limits.get_or_insert(other);
        assert_eq!(restored.cache_limits, Some(first));
        assert!(!restored.scopes_visible);
        restored.cache_limits = Some(CacheLimits {
            cpu_bytes: 123 * MIB,
            gpu_bytes: 0,
        });
        store.save(&restored).unwrap();
        let (_, mut reopened) = Store::open(path.clone()).unwrap();
        reopened.cache_limits.get_or_insert(other);
        assert_eq!(reopened.cache_limits, restored.cache_limits);
        // Explicit reset, unlike a relaunch, deliberately selects fresh defaults.
        reopened.cache_limits = Some(other);
        store.save(&reopened).unwrap();
        assert_eq!(Store::open(path).unwrap().1.cache_limits, Some(other));
    }

    #[test]
    fn invalid_changes_do_not_replace_the_last_saved_preferences() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        let (mut store, mut preferences) = Store::open(path.clone()).unwrap();
        store.save(&preferences).unwrap();
        let before = std::fs::read(&path).unwrap();
        preferences.filmstrip_height = f32::NAN;
        assert!(store.save(&preferences).is_err());
        assert_eq!(std::fs::read(path).unwrap(), before);
    }

    #[test]
    fn malformed_future_and_invalid_preferences_are_reported_and_left_intact() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        for content in [
            "not json",
            r#"{"version":2}"#,
            r#"{"filmstrip_height":-1}"#,
            r#"{"scope_height_ratio":0}"#,
            r#"{"cache_limits":{"cpu_bytes":-1,"gpu_bytes":0}}"#,
            r#"{"cache_limits":{"cpu_bytes":1,"gpu_bytes":0}}"#,
            r#"{"cache_limits":{"cpu_bytes":1048576}}"#,
            r#"{"cache_limits":{"cpu_bytes":0,"gpu_bytes":0,"future":1}}"#,
        ] {
            std::fs::write(&path, content).unwrap();
            assert!(Store::open(path.clone()).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), content);
        }
    }
}
