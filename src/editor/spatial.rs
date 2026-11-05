//! Document-owned region preparation, awaited independently by feature actions.
#[cfg(test)]
use super::ActivityReporter;
use super::{ActivityOptions, Document, Editor, UiContext, document};
use crate::{
    cancellation::Cancellation,
    spatial_metadata::{Metadata, Request},
};
use futures_channel::oneshot;
use std::sync::Arc;

pub(super) mod preparation;

#[cfg(test)]
mod tests;

pub(super) struct Pending {
    #[cfg(test)]
    pub activity: ActivityReporter,
    cancelled: Cancellation,
    completion: Vec<oneshot::Sender<Option<Arc<Metadata>>>>,
    task: Option<async_executor::Task<()>>,
}

#[cfg(test)]
impl Pending {
    pub fn visible(&self) -> bool {
        self.activity.view().is_some()
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.cancelled.cancel();
    }
}

impl Editor {
    pub(super) fn prepare_regions<'scope>(
        &mut self,
        cx: &UiContext<'scope, Self>,
    ) -> Option<impl Future<Output = Option<Arc<Metadata>>> + use<'scope>> {
        self.prepare_region_inputs(cx, false)
    }

    pub(super) fn prepare_analysis_regions<'scope>(
        &mut self,
        cx: &UiContext<'scope, Self>,
    ) -> Option<impl Future<Output = Option<Arc<Metadata>>> + use<'scope>> {
        self.prepare_region_inputs(cx, true)
    }

    fn prepare_region_inputs<'scope>(
        &mut self,
        cx: &UiContext<'scope, Self>,
        sampling: bool,
    ) -> Option<impl Future<Output = Option<Arc<Metadata>>> + use<'scope>> {
        if self.interaction_blocked() && self.document.spatial_preparing.is_none() {
            return None;
        }
        self.sync_document();
        let (send, receive) = oneshot::channel();
        let document = Arc::new(self.document.capture());
        let ready = document.ready_spatial_metadata();
        let inputs = document.spatial_inputs();
        let complete = ready.groups.iter().map(|g| g.sources.len()).sum::<usize>() == inputs.len()
            && document
                .project
                .order
                .iter()
                .all(|id| inputs.iter().any(|input| input.source == *id));
        let pixels_ready = !sampling
            || inputs.iter().all(|input| {
                document.source_key(input.source).is_ok_and(|key| {
                    self.document
                        .sampling_inputs
                        .get(input.source, &key)
                        .is_some()
                })
            });

        if self.document.spatial_preparing.is_none() && complete && pixels_ready {
            let _ = send.send(Some(Arc::new(ready)));
        } else if let Some(pending) = &mut self.document.spatial_preparing {
            pending.completion.push(send);
        } else {
            let request = self.document.spatial.begin()?;
            let cancelled = Cancellation::default();
            let activity = self.activities.start(
                ActivityOptions::new("Preparing film and picture regions…")
                    .modal()
                    .cancellable(
                        "Cancel region preparation",
                        "Cancelling region preparation…",
                        cancelled.clone(),
                    ),
            );
            let progress = activity.reporter();
            self.document.spatial_preparing = Some(Pending {
                #[cfg(test)]
                activity: activity.reporter(),
                cancelled: cancelled.clone(),
                completion: vec![send],
                task: None,
            });

            self.status = "Preparing film and picture regions…".into();
            let device = self
                .source_services
                .as_ref()
                .map(|source| source.device.clone());
            let sources = self.source_provider();
            let sampling_inputs = self.document.sampling_inputs.clone();
            let exposures = self.document.exposures.clone();

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
                            .unwrap_or_else(|_| Err("Programmed preparation dropped".into()))
                            .map(|metadata| {
                                metadata.map(|metadata| preparation::Prepared {
                                    metadata,
                                    sampling_inputs: Default::default(),
                                })
                            });
                    }
                    let device = device.ok_or_else(|| "GPU device unavailable".to_owned())?;
                    let sources =
                        sources.ok_or_else(|| "Source provider unavailable".to_owned())?;
                    preparation::prepare(
                        captured,
                        device,
                        sources,
                        sampling_inputs,
                        exposures,
                        cancelled.clone(),
                        move |done, total| progress.progress(done, total),
                    )
                    .await
                    .map_err(|error| error.to_string())
                };
                let stop = async {
                    cancelled.wait().await;
                    None
                };
                let result = futures_lite::future::or(async { Some(work.await) }, stop).await;

                drop(activity);
                cx.update(|editor| match result {
                    Some(result) => editor.regions_prepared(document, request, result),
                    None => editor.cancel_spatial(),
                });
            });
            if let Some(task) = task {
                self.document.spatial_preparing.as_mut().unwrap().task = Some(task);
            } else {
                self.cancel_spatial();
                self.status =
                    "Regions unavailable: Application stopped; accepted edits unchanged".into();
            }
        }
        let resumed = self.document.spatial_resumed.clone();
        let cx = cx.clone();
        Some(async move {
            let metadata = receive.await.ok().flatten()?;

            loop {
                let changed = resumed.listen();
                if cx.read(|editor| editor.document_pending.is_some() || editor.failure.is_some()) {
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
        if self.document.spatial_preparing.take().is_some() {
            self.document.spatial.cancel();
            if self.document_pending.is_none() {
                self.status = "Region preparation cancelled; accepted edits unchanged".into();
            }
        }
    }

    fn regions_prepared(
        &mut self,

        document: Arc<Document>,
        request: Request,
        result: Result<Option<preparation::Prepared>, String>,
    ) {
        let Some(mut pending) = self.document.spatial_preparing.take() else {
            return;
        };
        let foreground = self.document_pending.is_none();
        let result = result
            .and_then(|metadata| metadata.ok_or_else(|| "Region preparation cancelled".to_owned()));
        let metadata = match result {
            Ok(metadata) => metadata,
            Err(error) => {
                self.document.spatial.cancel();
                if foreground {
                    self.status = format!("Regions unavailable: {error}; accepted edits unchanged");
                }
                return;
            }
        };
        let preparation::Prepared {
            metadata,
            sampling_inputs,
        } = metadata;
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
            if foreground {
                self.status = "Regions discarded: source inputs changed".into();
            }
            return;
        }
        scope.sampling_inputs = sampling_inputs;
        log::info!(bytes = scope.sampling_inputs.retained_bytes(); "analysis_inputs_prepared");
        for completion in pending.completion.drain(..) {
            let _ = completion.send(Some(self.document.spatial.ready()));
        }
    }

    pub(super) fn resume_region_actions(&self) {
        self.document.spatial_resumed.notify(usize::MAX);
    }
}
