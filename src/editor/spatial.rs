//! Document-owned region preparation, awaited independently by feature actions.
use super::{
    Document, Editor, Task, TaskProgress, TaskRequest, UiContext,
    analysis_services::{Cancellation, Services},
    document,
};
use crate::spatial_metadata::{Metadata, Request};
use futures_channel::oneshot;
use std::sync::{Arc, Mutex};

mod preparation;

#[cfg(test)]
mod tests;

pub(super) struct Pending {
    pub id: u64,
    cancelled: Cancellation,
    completion: Option<oneshot::Sender<Option<Arc<Metadata>>>>,
    task: Option<async_executor::Task<()>>,
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
    pub(super) fn prepare_regions<'scope>(
        &mut self,
        cx: &UiContext<'scope, Self>,
    ) -> Option<impl Future<Output = Option<Arc<Metadata>>> + use<'scope>> {
        if self.busy || self.document.spatial_preparing.is_some() {
            return None;
        }
        self.sync_document();
        let request = self.document.spatial.begin()?;
        let document = Arc::new(self.document.capture());
        let id = self.request_id();
        let cancelled = Cancellation::default();
        let (send, receive) = oneshot::channel();
        self.document.spatial_preparing = Some(Pending {
            id,
            cancelled: cancelled.clone(),
            completion: Some(send),
            task: None,
        });
        self.tasks.begin(Task::Regions(id));
        self.busy = true;
        self.status = "Preparing film and picture regions…".into();
        let services = self.source_services.as_ref().map(|source| {
            Services::new(
                source,
                source.provider(Some(self.document.cache.clone())),
                self.document.exposures.clone(),
            )
        });
        let progress = Arc::new(Progress::default());
        let produced = progress.clone();
        let retire = cancelled.cancel_on_drop();
        let captured = document.clone();
        #[cfg(test)]
        let programmed = self.document.spatial_reply.take();
        let task = cx.spawn(move |cx| async move {
            let _retire = retire;
            let work = async {
                #[cfg(test)]
                if let Some(programmed) = programmed {
                    return programmed
                        .await
                        .unwrap_or_else(|_| Err("Programmed preparation dropped".into()));
                }
                let services = services.ok_or_else(|| "Source services unavailable".to_owned())?;
                preparation::prepare(captured, services, cancelled, move |done, total| {
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
            cx.update(|editor| editor.regions_prepared(id, document, request, result));
        });
        if let Some(task) = task {
            self.document.spatial_preparing.as_mut().unwrap().task = Some(task);
        } else {
            self.cancel_spatial();
            self.status =
                "Regions unavailable: Application stopped; accepted edits unchanged".into();
        }
        let resumed = self.document.spatial_resumed.clone();
        let cx = cx.clone();
        Some(async move {
            let metadata = receive.await.ok().flatten()?;
            loop {
                let changed = resumed.listen();
                if cx.read(|editor| {
                    editor.busy || editor.document_pending.is_some() || editor.failure.is_some()
                }) {
                    changed.await;
                    continue;
                }
                return cx
                    .update(|editor| {
                        if editor.document.capture().ready_spatial_metadata() == *metadata {
                            Some(metadata)
                        } else {
                            editor.status = "Regions no longer match the supporting inputs".into();
                            None
                        }
                    })
                    .flatten();
            }
        })
    }

    pub(super) fn cancel_spatial(&mut self) {
        if let Some(pending) = self.document.spatial_preparing.take() {
            self.document.spatial.cancel();
            self.tasks.finish(TaskRequest::Regions(pending.id));
            self.busy = self.document_pending.is_some();
            self.status = "Region preparation cancelled; accepted edits unchanged".into();
        }
    }

    fn regions_prepared(
        &mut self,
        id: u64,
        document: Arc<Document>,
        request: Request,
        result: Result<Option<Metadata>, String>,
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
        let paths_match = metadata.groups.iter().flat_map(|g| &g.sources).all(|row| {
            match (
                self.document.source_path(row.input.source),
                document.source_path(row.input.source),
            ) {
                (Ok(current), Ok(captured)) => document::same_path(&current, &captured),
                _ => false,
            }
        });
        // Supporting peers must still match when their shared geometry is published.
        let current = self.document.spatial_inputs();
        let scope = &mut self.document;
        let publish = paths_match
            && scope.spatial_code_scale() == document.spatial_code_scale()
            && scope
                .spatial
                .publish(request, metadata, &scope.state.project, &current)
                .unwrap_or(false);
        if !publish {
            if !self.busy {
                self.status = "Regions discarded: source inputs changed".into();
            }
            return;
        }
        if let Some(completion) = pending.completion.take() {
            let _ = completion.send(Some(self.document.spatial.ready()));
        }
    }

    pub(super) fn resume_region_actions(&self) {
        self.document.spatial_resumed.notify(usize::MAX);
    }
}
