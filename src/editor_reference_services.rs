use super::{Document, Prepared, Source, SourceId, document_services, flat::FlatPreview, source};
use crate::{Error, raw::Rect};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(super) enum Operation {
    Preview(Box<source::Request>),
    Accept(Box<Acceptance>),
}

pub(super) struct Acceptance {
    pub region: Rect,
    pub document: Document,
    pub selected: Option<SourceId>,
    pub source: source::Context,
    pub record: Source,
    pub correction: Option<Arc<super::Correction>>,
}

impl Acceptance {
    pub fn estimate(mut self) -> Result<document_services::Request, Error> {
        if self.source.correction.is_some() {
            return Err(Error::InvalidParameter(
                "flat-field reference must be uncorrected",
            ));
        }
        let mut scan = self.source.scan()?;
        let map = Arc::new(scan.estimate_flat_field(self.region)?);
        let reference = self.document.register_reference(
            self.record,
            scan.source().metadata().development(scan.fixed_wb),
        )?;
        let enabled = self
            .document
            .project
            .flat_field
            .as_ref()
            .is_none_or(|field| field.enabled);
        let mut settings = crate::project::FlatFieldSettings::new(
            self.document.project.source(reference)?,
            map,
            self.document
                .project
                .development
                .as_ref()
                .ok_or(Error::UnusableSample("flat-field roll development missing"))?,
        );
        settings.enabled = enabled;
        self.document.project.flat_field = Some(settings);
        Ok(document_services::Request {
            work: document_services::Preparation::Correction(self.document, self.selected),
            source: None,
            correction: self.correction,
        })
    }
}

#[derive(Clone)]
pub(super) struct Services(pub source::Services);

impl Services {
    pub async fn preview(
        self,
        request: source::Request,
        cancelled: Arc<AtomicBool>,
    ) -> Result<FlatPreview, String> {
        if request.correction.is_some() {
            return Err("Flat-field reference must be uncorrected".into());
        }
        let ready = self.0.clone().prepare(request, cancelled.clone()).await?;
        self.0
            .jobs
            .run(move || {
                if cancelled.load(Ordering::Relaxed) {
                    return Err(Error::UnusableSample("reference retired"));
                }
                let scan = source::Context {
                    source: ready.developed.clone(),
                    correction: None,
                }
                .scan()?;
                let region = scan.propose_flat_field_area()?;
                Ok(FlatPreview { ready, region })
            })
            .await
            .map_err(|_| "Reference support worker panicked".to_owned())?
            .map_err(|error: Error| error.to_string())
    }

    pub async fn accept(
        self,
        request: Acceptance,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Prepared, String> {
        let retired = cancelled.clone();
        let request = self
            .0
            .jobs
            .run(move || {
                if retired.load(Ordering::Relaxed) {
                    return Err(Error::UnusableSample("reference retired"));
                }
                request.estimate()
            })
            .await
            .map_err(|_| "Reference estimation worker panicked".to_owned())?
            .map_err(|error| error.to_string())?;
        document_services::PreparationServices(self.0)
            .prepare(request, cancelled, |_, _| {})
            .await
    }
}
