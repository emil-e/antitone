//! Captured physical inputs. Callers request a source or proxy; resolution and storage stay here.
use super::{FileKey, Priority, SourceKey, SourceProvider};
use crate::{
    Error,
    development_metadata::Development,
    exposure_reference::ReferenceExposure,
    photo_preparation::PreparedSource,
    processing::InputSettings,
    project::{DevelopmentMode, ScanBasis, Source},
    scan::CameraPixels,
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

#[derive(Clone, Debug)]
enum Definition {
    Photo(Arc<Photograph>),
    Reference {
        white_balance: Option<[f64; 4]>,
        header: Arc<OnceLock<(Source, FileKey)>>,
    },
}

#[derive(Debug)]
pub(crate) struct Photograph {
    pub record: Source,
    pub development: Development,
    pub basis: Arc<OnceLock<ScanBasis>>,
    pub expected_camera: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct Input {
    path: PathBuf,
    file: FileKey,
    input: InputSettings,
    reference: Arc<OnceLock<ReferenceExposure>>,
    definition: Definition,
}

pub(crate) struct Acquired {
    pub prepared: PreparedSource,
    pub record: Source,
    pub file: FileKey,
    #[cfg(test)]
    capture_observer: Option<Arc<dyn Fn() + Send + Sync>>,
}

/// Original camera sites and the development that supplied them; no full-image lease is retained.
#[derive(Clone)]
pub(crate) struct CameraSamples {
    key: SourceKey,
    development: crate::project::SourceDevelopment,
    pixels: Arc<crate::scan::AnalysisInput>,
}

impl CameraSamples {
    #[cfg(test)]
    pub fn authored(
        key: SourceKey,
        development: crate::project::SourceDevelopment,
        pixels: Arc<crate::scan::AnalysisInput>,
    ) -> Self {
        Self {
            key,
            development,
            pixels,
        }
    }
    pub fn key(&self) -> &SourceKey {
        &self.key
    }
    pub fn development(&self) -> &crate::project::SourceDevelopment {
        &self.development
    }
    pub fn pixels(&self) -> Arc<crate::scan::AnalysisInput> {
        self.pixels.clone()
    }
    pub fn file(&self) -> &FileKey {
        self.key.file()
    }
}

impl Acquired {
    pub async fn camera_samples(
        self,
        cancelled: &crate::cancellation::Cancellation,
    ) -> Result<Option<CameraSamples>, Error> {
        let key = SourceKey::for_load(&self.file.path, self.file.clone(), &self.record.development);
        let development = self.record.development;
        let scan = self.prepared.scan();
        let stopped = cancelled.clone();
        #[cfg(test)]
        let observer = self.capture_observer;
        let pixels = crate::background::compute(cancelled, move || {
            #[cfg(test)]
            if let Some(observer) = observer {
                observer();
            }
            crate::scan::AnalysisInput::capture(&scan, || stopped.is_cancelled())
                .map(|pixels| pixels.map(Arc::new))
        })
        .await?
        .flatten();
        Ok(pixels.map(|pixels| CameraSamples {
            key,
            development,
            pixels,
        }))
    }
}

impl Input {
    pub fn photo(
        path: PathBuf,
        file: FileKey,
        input: InputSettings,
        reference: Arc<OnceLock<ReferenceExposure>>,
        photo: Photograph,
    ) -> Self {
        Self {
            path,
            file,
            input,
            reference,
            definition: Definition::Photo(Arc::new(photo)),
        }
    }

    pub fn reference(
        path: PathBuf,
        file: FileKey,
        white_balance: Option<[f64; 4]>,
        input: InputSettings,
        reference: Arc<OnceLock<ReferenceExposure>>,
    ) -> Self {
        Self {
            path,
            file,
            input,
            reference,
            definition: Definition::Reference {
                white_balance,
                header: Arc::default(),
            },
        }
    }

    pub fn same_request(&self, other: &Self) -> bool {
        self.path == other.path
            && self.file == other.file
            && DevelopmentMode::for_input(self.input) == DevelopmentMode::for_input(other.input)
            && match (&self.definition, &other.definition) {
                (Definition::Photo(a), Definition::Photo(b)) => a.development == b.development,
                (
                    Definition::Reference {
                        white_balance: a, ..
                    },
                    Definition::Reference {
                        white_balance: b, ..
                    },
                ) => a == b,
                _ => false,
            }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
    #[cfg(test)]
    pub fn input(&self) -> InputSettings {
        self.input
    }
    pub fn reference_exposure(&self) -> Arc<OnceLock<ReferenceExposure>> {
        self.reference.clone()
    }

    pub fn development(&self) -> Option<Development> {
        match &self.definition {
            Definition::Photo(photo) => Some(photo.development.clone()),
            Definition::Reference { .. } => None,
        }
    }

    pub fn record(&self) -> Result<Source, Error> {
        match &self.definition {
            Definition::Photo(photo) => {
                let mut record = photo.record.clone();
                record.development = photo.development.resolved()?.clone();
                record.development_mode = photo.development.mode();
                Ok(record)
            }
            Definition::Reference { header, .. } => header
                .get()
                .map(|(record, _)| record.clone())
                .ok_or(Error::UnusableSample("source has not been acquired")),
        }
    }

    pub fn cached_key(&self) -> Option<SourceKey> {
        let record = self.record().ok()?;
        let file = match &self.definition {
            Definition::Photo(_) => &self.file,
            Definition::Reference { header, .. } => &header.get()?.1,
        };
        Some(SourceKey::for_load(
            &self.path,
            file.clone(),
            &record.development,
        ))
    }

    async fn key(&self) -> Result<SourceKey, Error> {
        match &self.definition {
            Definition::Photo(photo) => {
                let development = photo.development.acquire().await?;
                if self.input.requires_scan_sources() {
                    if photo
                        .expected_camera
                        .as_ref()
                        .is_some_and(|camera| *camera != development.camera_model)
                    {
                        return Err(Error::Project("sources do not share one scanning setup; use Digital for mixed sources".into()));
                    }
                    photo
                        .basis
                        .get_or_init(|| ScanBasis::from_development(&development))
                        .require_source(&development)?;
                }
                Ok(SourceKey::for_load(
                    &self.path,
                    self.file.clone(),
                    &development,
                ))
            }
            Definition::Reference {
                white_balance,
                header,
            } => {
                if header.get().is_none() {
                    let path = self.path.clone();
                    let white_balance = *white_balance;
                    let (mut record, file) = crate::background::run(move |_| {
                        let (record, raw) = super::open_source(&path, white_balance)?;
                        Ok::<_, Error>((record, raw.file.clone()))
                    })
                    .await
                    .map_err(|_| Error::UnusableSample("source header worker panicked"))??;
                    record.development_mode = DevelopmentMode::for_input(self.input);
                    let _ = header.set((record, file));
                }
                let (record, file) = header.get().expect("resolved reference header");
                Ok(SourceKey::for_load(
                    &self.path,
                    file.clone(),
                    &record.development,
                ))
            }
        }
    }

    pub async fn source(
        &self,
        provider: &dyn SourceProvider,
        priority: Priority,
    ) -> Result<Acquired, Error> {
        let key = self
            .key()
            .await
            .map_err(|error| Error::Raw(format!("{}: {error}", self.path.display())))?;
        let file = key.file().clone();
        let prepared = crate::photo_preparation::acquire(
            provider,
            key,
            priority,
            self.input,
            self.reference.clone(),
        )
        .await?;
        let record = self.record()?;
        super::verify_scan(&record, &prepared.scan())?;
        if let Definition::Photo(photo) = &self.definition {
            photo.development.observe_preview(
                prepared
                    .scan()
                    .source()
                    .metadata()
                    .source_preview(prepared.scan().source().white_balance()),
            );
        }
        Ok(Acquired {
            prepared,
            record,
            file,
            #[cfg(test)]
            capture_observer: provider.camera_capture_observer(),
        })
    }

    pub async fn camera_samples(
        &self,
        provider: &dyn SourceProvider,
        cached: Option<(SourceKey, Arc<crate::scan::AnalysisInput>)>,
        cancelled: &crate::cancellation::Cancellation,
    ) -> Result<Option<CameraSamples>, Error> {
        if cancelled.is_cancelled() {
            return Ok(None);
        }
        let key = futures_lite::future::or(
            async {
                cancelled.wait().await;
                Ok(None)
            },
            async {
                self.key()
                    .await
                    .map(Some)
                    .map_err(|error| Error::Raw(format!("{}: {error}", self.path.display())))
            },
        )
        .await?;
        let Some(key) = key else {
            return Ok(None);
        };
        if let Some((stored, pixels)) = cached
            && stored == key
        {
            let development = self.record()?.development;
            if pixels.area() != development.active_area.into() {
                return Err(Error::InvalidParameter(
                    "captured camera grid differs from source",
                ));
            }
            return Ok((!cancelled.is_cancelled()).then_some(CameraSamples {
                key,
                development,
                pixels,
            }));
        }
        let acquired = futures_lite::future::or(
            async {
                cancelled.wait().await;
                Ok(None)
            },
            async { self.source(provider, Priority::Foreground).await.map(Some) },
        )
        .await?;
        let Some(acquired) = acquired else {
            return Ok(None);
        };
        acquired.camera_samples(cancelled).await
    }

    pub async fn proxy(
        &self,
        provider: &dyn SourceProvider,
        priority: Priority,
    ) -> Result<Arc<CameraPixels>, Error> {
        let key = self.key().await?;
        let proxy = provider.load_proxy(key, priority).await?;
        if let Definition::Photo(photo) = &self.definition {
            photo.development.acquire_preview().await?;
        }
        Ok(proxy)
    }

    pub async fn positive_proxy(
        &self,
        provider: &dyn SourceProvider,
        priority: Priority,
        cached: Option<Arc<CameraPixels>>,
    ) -> Result<crate::scan::CameraProxy, Error> {
        let key = self.key().await?;
        let pixels = match cached {
            Some(pixels) => pixels,
            None => provider.load_proxy(key.clone(), priority).await?,
        };
        if !key.matches_proxy(&pixels) {
            return Err(Error::Raw("proxy differs from requested source".into()));
        }
        let preview = if !self.input.is_negative() && self.reference.get().is_none() {
            provider.load_preview(key).await
        } else {
            Ok(None)
        };
        let input = self.input;
        let reference = self.reference.clone();
        crate::background::run(move |_| {
            crate::photo_preparation::proxy(&reference, input, &pixels, preview)
        })
        .await
        .map_err(|_| Error::UnusableSample("proxy preparation worker panicked"))
    }

    pub async fn preview(&self, provider: &dyn SourceProvider) -> super::PreviewResult {
        let key = self.key().await.map_err(|error| error.to_string())?;
        provider.load_preview(key).await
    }

    pub fn matches(&self, path: &Path, file: &FileKey, record: &Source) -> bool {
        self.path == path
            && self
                .record()
                .is_ok_and(|expected| expected.development.same_processing(&record.development))
            && match &self.definition {
                Definition::Photo(_) => &self.file == file,
                Definition::Reference { header, .. } => {
                    header.get().is_some_and(|(_, expected)| expected == file)
                }
            }
    }
}
