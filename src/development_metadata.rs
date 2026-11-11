//! Recorded interpretation-specific developments, independent of optional pixel retention.
use crate::{
    Error,
    processing::InputSettings,
    project::{DevelopmentMode, Project, ScanBasis, Source, SourceDevelopment, SourceId},
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub source: SourceId,
    pub mode: DevelopmentMode,
    pub development: SourceDevelopment,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub entries: Vec<Entry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan_basis: Option<ScanBasis>,
}

impl Metadata {
    pub fn validate(&self, project: &Project) -> Result<(), Error> {
        let mut seen = std::collections::HashSet::new();
        for entry in &self.entries {
            if !seen.insert((entry.source, entry.mode)) {
                return Err(Error::Project("duplicate recorded development".into()));
            }
            let source = project.source(entry.source)?;
            entry.development.validate()?;
            require_geometry(&source.development, &entry.development)?;
            if entry.mode == source.development_mode
                && !entry.development.same_processing(&source.development)
            {
                return Err(Error::Project(
                    "recorded development differs from the registered baseline".into(),
                ));
            }
            if entry.mode == DevelopmentMode::Unity
                && entry.development.decode.white_balance != crate::project::UNITY_WB
            {
                return Err(Error::Project(
                    "recorded scan development requires unity WB".into(),
                ));
            }
        }
        if let Some(basis) = &self.scan_basis {
            basis.validate()?;
        }
        Ok(())
    }

    pub fn for_project(&self, project: &Project) -> Self {
        Self {
            entries: self
                .entries
                .iter()
                .filter(|entry| project.source(entry.source).is_ok())
                .cloned()
                .collect(),
            scan_basis: self.scan_basis.clone(),
        }
    }
}

pub(crate) fn require_geometry(
    declared: &SourceDevelopment,
    actual: &SourceDevelopment,
) -> Result<(), Error> {
    if declared.dimensions != actual.dimensions
        || declared.active_area != actual.active_area
        || declared.camera_model != actual.camera_model
    {
        return Err(Error::Project(
            "source geometry or camera differs from the registered photograph".into(),
        ));
    }
    Ok(())
}

/// A captured interpretation and its once-recorded result. Equality describes the request,
/// never the changing availability of the resolution.
#[derive(Clone, Debug)]
pub struct Development {
    definition: Definition,
    value: Arc<OnceLock<SourceDevelopment>>,
    #[cfg(feature = "editor")]
    preview: Arc<OnceLock<crate::raw::SourcePreview>>,
    #[cfg(feature = "editor")]
    resolving: Arc<std::sync::Mutex<Option<Resolution>>>,
}

#[derive(Clone, Debug, PartialEq)]
struct Definition {
    source: SourceId,
    origin: PathBuf,
    registered: SourceDevelopment,
    mode: DevelopmentMode,
}

#[cfg(feature = "editor")]
type Resolution = futures_util::future::Shared<
    futures_util::future::BoxFuture<'static, Result<SourceDevelopment, Error>>,
>;

impl PartialEq for Development {
    fn eq(&self, other: &Self) -> bool {
        self.definition.source == other.definition.source
            && self.definition.origin == other.definition.origin
            && self.definition.registered == other.definition.registered
            && self.definition.mode == other.definition.mode
    }
}

impl Development {
    pub fn captured(
        source: &Source,
        origin: &Path,
        input: InputSettings,
        metadata: &Metadata,
    ) -> Self {
        let mode = DevelopmentMode::for_input(input);
        let recorded = (source.development_mode == mode)
            .then(|| source.development.clone())
            .or_else(|| {
                metadata
                    .entries
                    .iter()
                    .find(|entry| entry.source == source.id && entry.mode == mode)
                    .map(|entry| entry.development.clone())
            });
        Self::new(
            source.id,
            origin.to_owned(),
            source.development.clone(),
            mode,
            recorded,
        )
    }

    pub fn fixed(development: SourceDevelopment) -> Self {
        Self::new(
            SourceId(0),
            PathBuf::new(),
            development.clone(),
            DevelopmentMode::Unity,
            Some(development),
        )
    }

    fn new(
        source: SourceId,
        origin: PathBuf,
        registered: SourceDevelopment,
        mode: DevelopmentMode,
        recorded: Option<SourceDevelopment>,
    ) -> Self {
        let value = Arc::new(recorded.clone().map_or_else(OnceLock::new, OnceLock::from));
        Self {
            definition: Definition {
                source,
                origin,
                registered,
                mode,
            },
            value,
            #[cfg(feature = "editor")]
            preview: Arc::default(),
            #[cfg(feature = "editor")]
            resolving: Arc::default(),
        }
    }

    pub fn resolved(&self) -> Result<&SourceDevelopment, Error> {
        self.value.get().ok_or(Error::UnusableSample(
            "source interpretation has not been acquired",
        ))
    }

    pub fn registered(&self) -> &SourceDevelopment {
        &self.definition.registered
    }
    pub fn mode(&self) -> DevelopmentMode {
        self.definition.mode
    }
    pub fn active_area(&self) -> [u32; 4] {
        self.definition.registered.active_area
    }

    #[cfg(feature = "editor")]
    pub(crate) async fn acquire(&self) -> Result<SourceDevelopment, Error> {
        if let Some(value) = self.value.get() {
            return Ok(value.clone());
        }
        self.resolve_header().await
    }

    #[cfg(feature = "editor")]
    pub(crate) fn source_preview(&self) -> Option<crate::raw::SourcePreview> {
        self.preview.get().copied()
    }

    #[cfg(feature = "editor")]
    pub(crate) fn observe_preview(&self, preview: crate::raw::SourcePreview) {
        let _ = self.preview.set(preview);
    }

    #[cfg(feature = "editor")]
    pub(crate) async fn acquire_preview(&self) -> Result<(), Error> {
        if self.preview.get().is_none() {
            self.resolve_header().await?;
        }
        Ok(())
    }

    #[cfg(feature = "editor")]
    async fn resolve_header(&self) -> Result<SourceDevelopment, Error> {
        use futures_util::FutureExt;
        let resolution = {
            let mut resolving = self.resolving.lock().unwrap();
            resolving
                .get_or_insert_with(|| {
                    let path = self.definition.origin.clone();
                    let mode = self.definition.mode;
                    let registered = self.definition.registered.clone();
                    let value = self.value.clone();
                    let known = value.get().cloned();
                    let preview = self.preview.clone();
                    async move {
                        let (development, viewing) = crate::background::run(move |_| {
                            let wb = known
                                .as_ref()
                                .map(|development| development.decode.white_balance)
                                .or_else(|| mode.white_balance());
                            let (source, raw) = crate::source::open_source(&path, wb)?;
                            require_geometry(&registered, &source.development)?;
                            if known
                                .as_ref()
                                .is_some_and(|known| !known.same_processing(&source.development))
                            {
                                return Err(Error::UnusableSample(
                                    "source viewing metadata differs from its recorded development",
                                ));
                            }
                            let viewing = raw
                                .metadata
                                .source_preview(source.development.decode.white_balance);
                            Ok::<_, Error>((source.development, viewing))
                        })
                        .await
                        .map_err(|_| {
                            Error::UnusableSample("source interpretation worker panicked")
                        })??;
                        let _ = preview.set(viewing);
                        let _ = value.set(development);
                        Ok(value
                            .get()
                            .expect("successful development resolution")
                            .clone())
                    }
                    .boxed()
                    .shared()
                })
                .clone()
        };
        let result = resolution.clone().await;
        if result.is_err() {
            let mut resolving = self.resolving.lock().unwrap();
            if resolving
                .as_ref()
                .is_some_and(|current| current.ptr_eq(&resolution))
            {
                resolving.take();
            }
        }
        result
    }
}

impl From<SourceDevelopment> for Development {
    fn from(value: SourceDevelopment) -> Self {
        Self::fixed(value)
    }
}

#[cfg(feature = "editor")]
#[derive(Default)]
pub(crate) struct Store {
    values: std::sync::Mutex<Vec<Development>>,
}

#[cfg(feature = "editor")]
impl Store {
    pub fn restored(
        metadata: &Metadata,
        project: &Project,
        origin: impl Fn(SourceId) -> Option<PathBuf>,
    ) -> Self {
        let store = Self::default();
        for entry in &metadata.entries {
            if let (Ok(source), Some(origin)) = (project.source(entry.source), origin(entry.source))
            {
                let input = if entry.mode == DevelopmentMode::Unity {
                    InputSettings::default()
                } else {
                    InputSettings::Digital
                };
                store.request(source, &origin, input, metadata);
            }
        }
        store
    }

    pub fn request(
        &self,
        source: &Source,
        origin: &Path,
        input: InputSettings,
        recorded: &Metadata,
    ) -> Development {
        let mode = DevelopmentMode::for_input(input);
        let mut values = self.values.lock().unwrap();
        if let Some(value) = values.iter().find(|value| {
            value.definition.source == source.id
                && value.definition.origin == origin
                && value.definition.registered == source.development
                && value.mode() == mode
        }) {
            return value.clone();
        }
        let value = Development::captured(source, origin, input, recorded);
        values.push(value.clone());
        value
    }

    pub fn snapshot(
        &self,
        project: &Project,
        origin: impl Fn(SourceId) -> Option<PathBuf>,
    ) -> Metadata {
        Metadata {
            entries: self
                .values
                .lock()
                .unwrap()
                .iter()
                .filter_map(|value| {
                    let definition = &value.definition;
                    let source = project.source(definition.source).ok()?;
                    if definition.mode == source.development_mode {
                        return None;
                    }
                    if origin(source.id).as_ref() != Some(&definition.origin)
                        || source.development != definition.registered
                    {
                        return None;
                    }
                    Some(Entry {
                        source: source.id,
                        mode: definition.mode,
                        development: value.value.get()?.clone(),
                    })
                })
                .collect(),
            scan_basis: None,
        }
    }
}
