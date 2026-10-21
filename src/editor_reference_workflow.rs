#[cfg(test)]
#[path = "editor_reference_workflow_tests.rs"]
mod tests;

use super::{
    Editor, FailureKind, Pick, TaskRequest,
    flat::FlatPreview,
    reference_services::{Operation, Services},
};
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
    pub(super) fn start_reference_workflow(
        &mut self,
        access: &crate::application::UiAccess<'_, Self>,
    ) {
        let Some(operation) = self
            .flat_edit
            .as_mut()
            .and_then(|session| session.request.take())
        else {
            return;
        };
        let id = self.id;
        let cancelled = Arc::new(AtomicBool::new(false));
        let retire = Retire(cancelled.clone());
        let services = self.source_services.clone().map(Services);
        let accepting = matches!(operation, Operation::Accept(_));
        let Some(services) = services else {
            if accepting {
                self.document_prepared(id, Err("Reference services unavailable".into()));
            } else {
                self.reference_previewed(id, Err("Reference services unavailable".into()));
            }
            return;
        };
        let target = access.clone();
        let Some(task) = access.spawn(async move {
            let _retire = retire;
            match operation {
                Operation::Preview(request) => {
                    let result = services.preview(*request, cancelled).await;
                    target.update(|editor| editor.reference_previewed(id, result));
                }
                Operation::Accept(request) => {
                    let result = services.accept(*request, cancelled).await.map(Box::new);
                    target.update(|editor| {
                        if editor.flat_edit.is_some() {
                            editor.document_prepared(id, result);
                        }
                        editor.start_source_workflow(&target);
                        editor.start_working_workflow(&target);
                        editor.start_original_workflow(&target);
                    });
                }
            }
        }) else {
            if accepting {
                self.document_prepared(id, Err("Application stopped".into()));
            } else {
                self.reference_previewed(id, Err("Application stopped".into()));
            }
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
        session.request = None;
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
