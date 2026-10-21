#[cfg(test)]
#[path = "editor_reference_workflow_tests.rs"]
mod tests;

use super::{
    Editor, FailureKind, Pick, TaskRequest,
    document_services::PreparationServices,
    flat::FlatPreview,
    reference_services::{Acceptance, Services},
    source,
};
use crate::application::UiContext;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

struct Retire(Arc<AtomicBool>);
impl Drop for Retire {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

impl Editor {
    pub(super) fn preview_reference(&mut self, request: source::Request, cx: &UiContext<'_, Self>) {
        let id = self.id;
        let services = self.source_services.clone().map(Services);
        let cancelled = Arc::new(AtomicBool::new(false));
        let retire = Retire(cancelled.clone());
        let Some(task) = cx.spawn(move |cx| async move {
            let _retire = retire;
            let result = match services {
                Some(services) => services.preview(request, cancelled).await,
                None => Err("Reference services unavailable".into()),
            };
            cx.update(|editor| editor.reference_previewed(id, result));
        }) else {
            self.reference_previewed(id, Err("Application stopped".into()));
            return;
        };
        self.flat_edit
            .as_mut()
            .expect("live reference session")
            .task = Some(task);
    }

    pub(super) fn accept_reference(&mut self, input: Acceptance, cx: &UiContext<'_, Self>) {
        let id = self.id;
        let services = self.source_services.clone().map(Services);
        let cancelled = Arc::new(AtomicBool::new(false));
        let retire = Retire(cancelled.clone());
        let Some(task) = cx.spawn(move |cx| async move {
            let _retire = retire;
            let result = async {
                let services =
                    services.ok_or_else(|| "Reference services unavailable".to_owned())?;
                let request = services.estimate(input, cancelled.clone()).await?;
                PreparationServices(services.0)
                    .prepare(request, cancelled, |_, _| {})
                    .await
            }
            .await
            .map(Box::new);
            cx.update(|editor| {
                if editor.flat_edit.is_none() || editor.document_pending != Some(id) {
                    return;
                }
                editor.document_prepared(id, result);
                editor.acquire_working(&cx);
            });
        }) else {
            self.document_prepared(id, Err("Application stopped".into()));
            return;
        };
        self.flat_edit
            .as_mut()
            .expect("live reference session")
            .task = Some(task);
    }

    pub(super) fn reference_previewed(&mut self, id: u64, result: Result<FlatPreview, String>) {
        if id != self.id
            || self.flat_edit.is_none()
            || self.document_pending.is_some()
            || self.pick != Some(Pick::Flat)
        {
            return;
        }
        let session = self.flat_edit.as_mut().unwrap();
        session.task = None;
        self.tasks.finish(TaskRequest::Reference(id));
        self.busy = false;
        match result {
            Ok(preview) => {
                session.reference = Some(preview);
                let reference = session.reference.as_ref().unwrap();
                self.region = self
                    .region
                    .or(reference.region.map(super::Coverage::rectangle));
                self.document.visit.ready = Some(reference.ready.clone());
                self.resolve_failure(FailureKind::FlatReference);
                self.status = if self.region.is_some() {
                    "Proposed illuminated rectangle: adjust edges, then Apply."
                } else {
                    "Illuminated area is ambiguous; draw a rectangle around the usable light."
                }
                .into();
            }
            Err(error) => {
                self.status = error.clone();
                self.finish_flat_edit();
                self.report_failure(FailureKind::FlatReference, error);
            }
        }
    }
}
