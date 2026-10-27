//! Document-owned shared preparation. Consumers own only the continuation into their tools.
use super::{
    Document, Editor, SourceId, Task, TaskProgress, TaskRequest, UiContext, analysis,
    analysis_services::{Cancellation, Services},
    auto_crop, document,
};
use crate::spatial_metadata::{Metadata, Request};
use std::sync::{Arc, Mutex};

mod preparation;

#[cfg(test)]
mod tests;

pub(super) enum Consumer {
    Analyze(Box<analysis::Request>),
    AutoCrop {
        target: auto_crop::Target,
        sources: Vec<SourceId>,
        visit: u64,
    },
}

pub(super) struct Pending {
    pub id: u64,
    document: Arc<Document>,
    request: Option<Request>,
    consumer: Option<Consumer>,
    cancelled: Cancellation,
    task: Option<async_executor::Task<()>>,
}

impl Pending {
    pub(super) fn analysis_request(&self) -> Option<&analysis::Request> {
        match self.consumer.as_ref() {
            Some(Consumer::Analyze(request)) => Some(request),
            Some(Consumer::AutoCrop { .. }) | None => None,
        }
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.cancelled.cancel();
    }
}

#[derive(Default)]
struct Progress {
    latest: Mutex<Option<(usize, usize)>>,
    changed: event_listener::Event,
}

impl Editor {
    pub(super) fn request_spatial(&mut self, consumer: Consumer, cx: &UiContext<'_, Self>) {
        if self.busy
            || self.document.spatial_preparing.is_some()
            || self.document.spatial_consumer.is_some()
        {
            return;
        }
        self.sync_document();
        let Some(request) = self.document.spatial.begin() else {
            return;
        };
        let document = Arc::new(self.document.capture());
        let id = self.request_id();
        let cancelled = Cancellation::default();
        self.document.spatial_preparing = Some(Pending {
            id,
            document: document.clone(),
            request: Some(request),
            consumer: Some(consumer),
            cancelled: cancelled.clone(),
            task: None,
        });
        self.tasks.begin(Task::Regions(id));
        self.busy = true;
        self.status = "Preparing film and picture regions…".into();
        let services = self.source_services.as_ref().map(Services::from);
        let progress = Arc::new(Progress::default());
        let produced = progress.clone();
        let retire = cancelled.cancel_on_drop();
        let Some(task) = cx.spawn(move |cx| async move {
            let _retire = retire;
            let work = async {
                let services = services.ok_or_else(|| "Source services unavailable".to_owned())?;
                preparation::prepare(document, services, cancelled, move |done, total| {
                    *produced.latest.lock().unwrap() = Some((done, total));
                    produced.changed.notify(usize::MAX);
                })
                .await
                .map_err(|error| error.to_string())
            };
            let reports = async {
                loop {
                    let changed = progress.changed.listen();
                    let latest = progress.latest.lock().unwrap().take();
                    if let Some((completed, total)) = latest {
                        cx.update(|editor| {
                            if editor
                                .document
                                .spatial_preparing
                                .as_ref()
                                .is_some_and(|p| p.id == id)
                            {
                                editor.tasks.report(
                                    TaskRequest::Regions(id),
                                    TaskProgress::Counted { completed, total },
                                );
                            }
                        });
                    }
                    changed.await;
                }
            };
            let result = futures_lite::future::or(work, reports).await;
            cx.update(|editor| editor.spatial_result(id, result, &cx));
        }) else {
            self.spatial_result(id, Err("Application stopped".into()), cx);
            return;
        };
        self.document
            .spatial_preparing
            .as_mut()
            .expect("registered spatial request")
            .task = Some(task);
    }

    pub(super) fn cancel_spatial(&mut self) {
        let pending = self.document.spatial_preparing.take();
        let consumer = self.document.spatial_consumer.take();
        if pending.is_some() || consumer.is_some() {
            self.document.spatial.cancel();
            if let Some(pending) = pending {
                self.tasks.finish(TaskRequest::Regions(pending.id));
            }
            self.busy = self.document_pending.is_some();
            self.status = "Region preparation cancelled; accepted edits unchanged".into();
        }
    }

    fn spatial_result(
        &mut self,
        id: u64,
        result: Result<Option<Metadata>, String>,
        cx: &UiContext<'_, Self>,
    ) {
        if !self
            .document
            .spatial_preparing
            .as_ref()
            .is_some_and(|p| p.id == id)
        {
            return;
        }
        let mut pending = self.document.spatial_preparing.take().unwrap();
        self.tasks.finish(TaskRequest::Regions(id));
        self.busy = self.document_pending.is_some();
        let result = result
            .and_then(|metadata| metadata.ok_or_else(|| "Region preparation cancelled".to_owned()));
        let metadata = match result {
            Ok(metadata) => metadata,
            Err(error) => {
                self.document.spatial.cancel();
                if !self.busy {
                    self.status = format!("Regions unavailable: {error}; accepted edits unchanged");
                }
                return;
            }
        };
        let inputs: Vec<_> = metadata
            .groups
            .iter()
            .flat_map(|g| g.sources.iter().map(|s| s.input.clone()))
            .collect();
        let paths_match = inputs.iter().all(|input| {
            match (
                self.document.source_path(input.source),
                pending.document.source_path(input.source),
            ) {
                (Ok(current), Ok(captured)) => document::same_path(&current, &captured),
                _ => false,
            }
        });
        // Worker completion can wait for the UI driver. Recheck cheap file identities at
        // publication so a changed supporting peer cannot lend stale dimensions to a consumer.
        let current = self.document.spatial_inputs();
        let scope = &mut self.document;
        let publish = paths_match
            && scope.spatial_code_scale() == pending.document.spatial_code_scale()
            && scope
                .spatial
                .publish(
                    pending.request.take().unwrap(),
                    metadata,
                    &scope.state.project,
                    &current,
                )
                .unwrap_or(false);
        if !publish {
            if !self.busy {
                self.status = "Regions discarded: source inputs changed".into();
            }
            return;
        }
        self.document.spatial_consumer = pending.consumer.take();
        self.resume_spatial_consumer(cx);
    }

    pub(super) fn resume_spatial_consumer(&mut self, cx: &UiContext<'_, Self>) {
        if self.busy || self.document_pending.is_some() || self.failure.is_some() {
            return;
        }
        let Some(consumer) = self.document.spatial_consumer.take() else {
            return;
        };
        if self.document.capture().ready_spatial_metadata() != *self.document.spatial.ready() {
            self.status = "Region consumer cancelled: supporting inputs changed".into();
            return;
        }
        match consumer {
            Consumer::Analyze(mut request) => {
                document::rebase_project(
                    &mut request.document.project,
                    request.document.path.as_deref(),
                    self.document.path.as_deref(),
                );
                if self.id != request.loaded
                    || self.current_source != Some(request.selected)
                    || self.document.project != request.document.project
                {
                    self.status = "Regions ready; analysis request no longer current".into();
                    return;
                }
                request.document = self.document.capture();
                self.prepare_analysis_review(*request, cx);
            }
            Consumer::AutoCrop {
                target,
                sources,
                visit,
            } => {
                if target == auto_crop::Target::Photo && self.document.visit.view != visit {
                    self.status = "Regions ready; original Crop visit has ended".into();
                    return;
                }
                self.auto_crop_sources(target, sources);
            }
        }
    }
}
