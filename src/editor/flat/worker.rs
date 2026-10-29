use super::FlatPreview;
use crate::{
    Error,
    cancellation::Cancellation,
    editor::{Document, Source, SourceId, document_services, source},
    raw::Rect,
};
use std::sync::Arc;

pub(super) struct CorrectionInput {
    pub region: Rect,
    pub sources: Option<Arc<dyn crate::source::SourceProvider>>,
    pub document: Document,
    pub selected: Option<SourceId>,
    pub source: source::Context,
    pub record: Source,
    pub correction: Option<Arc<crate::editor::Correction>>,
}

impl CorrectionInput {
    fn estimate(mut self) -> Result<document_services::Request, Error> {
        if self.source.correction.is_some() {
            return Err(Error::InvalidParameter(
                "flat-field reference must be uncorrected",
            ));
        }
        let mut scan = self.source.scan()?;
        let map = Arc::new(scan.estimate_flat_field(self.region)?);
        let reference = self.document.register_reference(self.record)?;
        let enabled = self
            .document
            .project
            .flat_field()
            .is_none_or(|field| field.enabled);
        let mut settings =
            crate::project::FlatFieldSettings::new(self.document.project.source(reference)?, map);
        settings.enabled = enabled;
        self.document.project.set_flat_field(Some(settings));
        let exposures = Arc::new(crate::exposure_metadata::Store::restored(
            &self.document.exposure_metadata,
            |id| self.document.exposure_origin(id),
        ));
        Ok(document_services::Request {
            work: document_services::Preparation::Correction(self.document, self.selected),
            source: None,
            sources: self.sources,
            exposures: Some(exposures),
            correction: self.correction,
        })
    }
}

pub(super) async fn preview(
    services: source::Services,
    request: source::Request,
    cancelled: Cancellation,
) -> Result<FlatPreview, String> {
    if request.correction.is_some() {
        return Err("Flat-field reference must be uncorrected".into());
    }
    let ready = services.prepare(request, cancelled.clone()).await?;
    crate::background::run(move |_| {
        if cancelled.is_cancelled() {
            return Err(Error::UnusableSample("reference retired"));
        }
        let scan = source::Context {
            source: ready.developed.clone(),
            reference: ready.reference.clone(),
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

pub(super) async fn estimate(
    mut input: CorrectionInput,
    cancelled: Cancellation,
) -> Result<document_services::Request, String> {
    let sources = input.sources.take();
    let mut result = crate::background::run(move |_| {
        if cancelled.is_cancelled() {
            return Err(Error::UnusableSample("reference retired"));
        }
        input.estimate()
    })
    .await
    .map_err(|_| "Reference estimation worker panicked".to_owned())?
    .map_err(|error| error.to_string())?;
    result.sources = sources;
    Ok(result)
}
