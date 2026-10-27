//! Captured document operations. Preparation produces candidates, never accepted
//! state; writes return exactly the baseline that reached the destination.
use super::{Correction, Document, Ready, history, source};
use crate::{
    Error,
    project::{Save, SourceId},
};
use std::{path::PathBuf, sync::Arc};

pub(super) enum Preparation {
    Import(Document, Vec<PathBuf>, Option<(u64, SourceId)>),
    Open(PathBuf),
    New,
    Correction(Document, Option<SourceId>),
    Input(Document, Option<SourceId>),
    History(Document, Option<SourceId>, history::Direction),
}

pub(super) struct Request {
    pub work: Preparation,
    pub source: Option<(u64, Ready)>,
    pub correction: Option<Arc<Correction>>,
    pub cache: Option<Arc<crate::source::SourceCache>>,
    pub exposures: Option<Arc<crate::exposure_metadata::Store>>,
}

pub(super) struct Prepared {
    pub document: Document,
    pub ready: Option<Ready>,
    pub selected: Option<SourceId>,
    pub reuse_ready: bool,
    pub correction: Option<Arc<Correction>>,
    pub preserve_view: bool,
}

pub(super) struct Write {
    pub document: Document,
    pub path: PathBuf,
    pub policy: Save,
}

impl Write {
    pub fn save(self, device: &wgpu::Device) -> Result<Document, Error> {
        self.document
            .save_with_device(&self.path, self.policy, Some(device))
    }
}

pub(super) struct Candidate {
    pub prepared: Prepared,
    pub source: Option<source::Request>,
}

/// The synchronous admission/preparation stage contains metadata, validation and
/// GPU preflight only. Selected native development is a separate awaited service.
pub(super) fn candidate(
    request: Request,
    device: &wgpu::Device,
    mut progress: impl FnMut(usize, usize),
) -> Result<Candidate, Error> {
    let Request {
        work,
        source: previous,
        correction: previous_correction,
        cache,
        exposures,
    } = request;
    let preserve_view = matches!(
        work,
        Preparation::Correction(..) | Preparation::Input(..) | Preparation::History(..)
    );
    let selected_override = match &work {
        Preparation::Correction(_, selected)
        | Preparation::Input(_, selected)
        | Preparation::History(_, selected, _) => Some(*selected),
        _ => None,
    };
    let (mut document, reuse) = match work {
        Preparation::Import(mut document, paths, reuse) => {
            document.import_with_progress(&paths, Some(device), &mut progress)?;
            (document, reuse)
        }
        Preparation::Open(path) => (
            Document::open_with_progress(&path, Some(device), progress)?,
            None,
        ),
        Preparation::New => (Document::default(), None),
        Preparation::Correction(document, _) => (document, None),
        Preparation::Input(mut document, _) => {
            let target = document.project.input;
            document.redevelop_for_input(target)?;
            (document, None)
        }
        Preparation::History(mut document, _, _) => {
            if let Some(exposures) = &exposures {
                document.capture_exposure(exposures);
            }
            document.validate_for_device(device)?;
            for id in document.project.order.clone() {
                document.refresh_source(id)?;
            }
            (document, None)
        }
    };
    if let Some(exposures) = &exposures {
        document.capture_exposure(exposures);
    }
    let correction = Correction::prepare(device, &document, previous_correction.as_ref())?;
    for record in &document.project.sources {
        crate::gpu::ProcessingSource::new(
            device,
            &[[0; 4]],
            glam::DMat3::from_cols_array_2d(&record.development.camera_to_ap1),
        )?;
    }
    if let Some((loaded, selected)) = reuse
        && let Some((_, ready)) = previous.filter(|(active, _)| *active == loaded)
        && document
            .project
            .source(selected)
            .is_ok_and(|source| source.development == ready.record.development)
        && (document.project.input.is_negative() || ready.reference.is_some())
    {
        return Ok(Candidate {
            prepared: Prepared {
                document,
                ready: Some(ready),
                selected: Some(selected),
                reuse_ready: true,
                correction,
                preserve_view,
            },
            source: None,
        });
    }
    let selected = selected_override.unwrap_or_else(|| {
        reuse
            .map(|(_, selected)| selected)
            .or_else(|| document.project.order.first().copied())
    });
    let source = selected
        .map(|id| {
            source_request(
                &document,
                id,
                correction.clone(),
                cache.clone(),
                exposures.as_deref(),
            )
        })
        .transpose()?;
    Ok(Candidate {
        prepared: Prepared {
            document,
            ready: None,
            selected,
            reuse_ready: false,
            correction,
            preserve_view,
        },
        source,
    })
}

fn source_request(
    document: &Document,
    id: SourceId,
    correction: Option<Arc<Correction>>,
    cache: Option<Arc<crate::source::SourceCache>>,
    exposures: Option<&crate::exposure_metadata::Store>,
) -> Result<source::Request, Error> {
    let path = document.source_path(id)?;
    let expected = document.project.source(id)?.clone();
    let wb = Some(expected.development.decode.fixed_wb);
    let file = document.file_key(&path);
    Ok(source::Request {
        reference: exposures.map_or_else(
            || {
                Arc::new(
                    document
                        .reference_exposure(&expected)
                        .cloned()
                        .map_or_else(std::sync::OnceLock::new, std::sync::OnceLock::from),
                )
            },
            |store| document.exposure_slot(store, id),
        ),
        input: document.project.input,
        path,
        expected: Some(expected),
        wb,
        cache,
        file,
        correction,
    })
}

#[derive(Clone)]
pub(super) struct PreparationServices(pub source::Services);

impl PreparationServices {
    pub async fn prepare(
        self,
        request: Request,
        cancelled: crate::cancellation::Cancellation,
        progress: impl FnMut(usize, usize) + Send + 'static,
    ) -> Result<Prepared, String> {
        let exposures = request.exposures.clone();
        let device = self.0.device.clone();
        let retired = cancelled.clone();
        let Candidate {
            mut prepared,
            source,
        } = crate::background::run(move || {
            if retired.is_cancelled() {
                return Err(Error::UnusableSample("document preparation retired"));
            }
            candidate(request, &device, progress)
        })
        .await
        .map_err(|_| "Document preparation worker panicked".to_owned())?
        .map_err(|error| error.to_string())?;
        if cancelled.is_cancelled() {
            return Err("Document preparation retired".into());
        }
        if let Some(source) = source {
            prepared.ready = Some(self.0.clone().prepare(source, cancelled).await?);
        }
        if let Some(id) = prepared.selected
            && let Some(ready) = &prepared.ready
            && let Some(reference) = &ready.reference
        {
            let metadata = Arc::make_mut(&mut prepared.document.exposure_metadata);
            metadata.entries.retain(|entry| entry.source != id);
            metadata.entries.push(crate::exposure_metadata::Entry {
                source: id,

                reference: reference.clone(),
            });
        }
        let device = self.0.device.clone();
        crate::background::run(move || {
            if let Some(exposures) = &exposures {
                prepared.document.capture_exposure(exposures);
            }
            prepared.document.validate_with_device(Some(&device))?;
            Ok(prepared)
        })
        .await
        .map_err(|_| "Document admission worker panicked".to_owned())?
        .map_err(|error: Error| error.to_string())
    }
}

#[derive(Clone)]
pub(super) struct Services {
    pub device: wgpu::Device,
}

impl Services {
    pub async fn write(self, write: Write) -> Result<Document, String> {
        crate::background::run(move || write.save(&self.device))
            .await
            .map_err(|_| "Document write worker panicked".to_owned())?
            .map_err(|error| error.to_string())
    }
}
