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
    History(Document, Option<SourceId>, history::Direction),
}

pub(super) struct Request {
    pub work: Preparation,
    pub source: Option<(u64, Ready)>,
    pub correction: Option<Arc<Correction>>,
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
    } = request;
    let preserve_view = matches!(work, Preparation::Correction(..) | Preparation::History(..));
    let selected_override = match &work {
        Preparation::Correction(_, selected) | Preparation::History(_, selected, _) => {
            Some(*selected)
        }
        _ => None,
    };
    let (document, reuse) = match work {
        Preparation::Import(mut document, paths, reuse) => {
            document.fork_cache();
            document.import_with_progress(&paths, Some(device), &mut progress)?;
            (document, reuse)
        }
        Preparation::Open(path) => (
            Document::open_with_progress(&path, Some(device), progress)?,
            None,
        ),
        Preparation::New => (Document::default(), None),
        Preparation::Correction(mut document, _) => {
            document.fork_cache();
            (document, None)
        }
        Preparation::History(mut document, _, _) => {
            document.fork_cache();
            document.validate_for_device(device)?;
            for id in &document.project.order {
                let (actual, _) = super::document::read_source(
                    &document.source_path(*id)?,
                    document.project.development.as_ref().map(|d| d.fixed_wb),
                )?;
                super::document::verify_source(document.project.source(*id)?, &actual)?;
            }
            (document, None)
        }
    };
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
    let selected = selected_override.unwrap_or_else(|| document.project.order.first().copied());
    let source = selected
        .map(|id| source_request(&document, id, correction.clone()))
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
) -> Result<source::Request, Error> {
    let path = document.source_path(id)?;
    let expected = document.project.source(id)?.clone();
    let wb = document.project.development.as_ref().map(|d| d.fixed_wb);
    let cache = source::Cache::new(&path, Some(&expected), wb, document);
    Ok(source::Request {
        path,
        expected: Some(expected),
        wb,
        basis: document.project.camera_basis.clone(),
        cache: Some(cache),
        correction,
    })
}

#[derive(Clone)]
pub(super) struct PreparationServices(pub source::Services);

impl PreparationServices {
    pub async fn prepare(
        self,
        request: Request,
        cancelled: Arc<std::sync::atomic::AtomicBool>,
        progress: impl FnMut(usize, usize) + Send + 'static,
    ) -> Result<Prepared, String> {
        let device = self.0.device.clone();
        let retired = cancelled.clone();
        let Candidate {
            mut prepared,
            source,
        } = self
            .0
            .jobs
            .run(move || {
                if retired.load(std::sync::atomic::Ordering::Relaxed) {
                    return Err(Error::UnusableSample("document preparation retired"));
                }
                candidate(request, &device, progress)
            })
            .await
            .map_err(|_| "Document preparation worker panicked".to_owned())?
            .map_err(|error| error.to_string())?;
        if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("Document preparation retired".into());
        }
        if let Some(source) = source {
            prepared.ready = Some(self.0.prepare(source, cancelled).await?);
        }
        Ok(prepared)
    }
}

#[derive(Clone)]
pub(super) struct Services {
    pub jobs: crate::blocking_jobs::BlockingJobs,
    pub device: wgpu::Device,
}

impl Services {
    pub async fn write(self, write: Write) -> Result<Document, String> {
        self.jobs
            .run(move || write.save(&self.device))
            .await
            .map_err(|_| "Document write worker panicked".to_owned())?
            .map_err(|error| error.to_string())
    }
}
