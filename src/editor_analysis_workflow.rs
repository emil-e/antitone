#[cfg(test)]
#[path = "editor_analysis_acknowledgement_tests.rs"]
mod acknowledgement_tests;
#[cfg(test)]
#[path = "editor_analysis_fit_workflow_tests.rs"]
mod fit_tests;
#[cfg(test)]
#[path = "editor_analysis_workflow_tests.rs"]
mod tests;

use super::{
    Editor, Error, Task, TaskProgress, TaskRequest, UiContext, analysis, analysis_review,
    analysis_services::{Cancellation, Services},
};
#[cfg(test)]
use std::sync::atomic::AtomicBool;
use std::{
    future::Future,
    sync::{Arc, Mutex, atomic::Ordering},
};

pub(super) struct Pending {
    pub id: u64,
    pub request: analysis::Request,
    cancelled: Cancellation,
    task: Option<async_executor::Task<()>>,
}

impl Pending {
    pub fn new(id: u64, request: analysis::Request) -> Self {
        Self {
            id,
            request,
            cancelled: Cancellation::default(),
            task: None,
        }
    }
    pub fn cancel(&self) {
        self.cancelled.cancel();
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[derive(Default)]
struct Progress {
    latest: Mutex<Option<(usize, usize)>>,
    changed: event_listener::Event,
}
impl Progress {
    fn report(&self, done: usize, total: usize) {
        *self.latest.lock().unwrap() = Some((done, total));
        self.changed.notify(usize::MAX);
    }
}

async fn drive<T>(
    work: impl Future<Output = Result<Option<T>, Error>>,
    progress: Arc<Progress>,
    cx: &UiContext<'_, Editor>,
    id: u64,
) -> Result<Option<T>, String> {
    let reports = async {
        loop {
            let changed = progress.changed.listen();
            let latest = progress.latest.lock().unwrap().take();
            if let Some((done, total)) = latest {
                cx.update(|editor| editor.analysis_progress(id, done, total));
            }
            changed.await;
        }
    };
    futures_lite::future::or(work, reports)
        .await
        .map_err(|error| error.to_string())
}

impl Editor {
    pub(super) fn prepare_analysis_review(
        &mut self,
        mut request: analysis::Request,
        cx: &UiContext<'_, Self>,
    ) {
        if self.document.visit.mask_edit.is_some()
            || self.document.auto_crop_edit.is_some()
            || self.document.visit.adjustment_layers.confirming()
        {
            return;
        }
        request.revision = self
            .document
            .analysis_generation
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        let id = self.request_id();
        self.thumbnails.cancel();
        self.tasks.begin(Task::AnalysisAreas(id));
        let pending = Pending::new(id, request.clone());
        let cancelled = pending.cancelled.clone();
        self.document.analysis_preparing = Some(pending);
        self.busy = true;
        self.status = "Preparing per-photo analysis areas…".into();
        let services = self.source_services.as_ref().map(Services::from);
        let progress = Arc::new(Progress::default());
        let produced = progress.clone();
        let retire = cancelled.cancel_on_drop();
        let Some(task) = cx.spawn(move |cx| async move {
            let _retire = retire;
            let result = match services {
                Some(services) => {
                    let preparation = analysis_review::preparation::prepare(
                        request,
                        services,
                        cancelled,
                        move |done, total| produced.report(done, total),
                    );
                    drive(preparation, progress, &cx, id).await
                }
                None => Err("Analysis services unavailable".into()),
            };
            cx.update(|editor| editor.analysis_areas_result(id, result, &cx));
        }) else {
            self.analysis_areas_result(id, Err("Application stopped".into()), cx);
            return;
        };
        self.document
            .analysis_preparing
            .as_mut()
            .expect("live area preparation")
            .task = Some(task);
    }

    pub(super) fn start_analysis(
        &mut self,
        mut request: analysis::Request,
        cx: &UiContext<'_, Self>,
    ) {
        let Some(input) = self.source_context() else {
            return;
        };
        request.revision = self
            .document
            .analysis_generation
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        let id = self.request_id();
        self.thumbnails.cancel();
        self.cancel_original_pending();
        self.tasks.begin(Task::Analyze(id));
        self.tasks.report(
            TaskRequest::Analyze(id),
            TaskProgress::Counted {
                completed: 0,
                total: request.document.project.order.len(),
            },
        );
        let pending = Pending::new(id, request.clone());
        let cancelled = pending.cancelled.clone();
        self.document.analysis_pending = Some(pending);
        self.busy = true;
        self.status = "Analyzing roll…".into();
        let services = self.source_services.as_ref().map(Services::from);
        let progress = Arc::new(Progress::default());
        let produced = progress.clone();
        let retire = cancelled.cancel_on_drop();
        let Some(task) = cx.spawn(move |cx| async move {
            let _retire = retire;
            let result = match services {
                Some(services) => {
                    let fitting = analysis::fit::run(
                        request,
                        input,
                        services,
                        cancelled,
                        move |done, total| produced.report(done, total),
                    );
                    drive(fitting, progress, &cx, id)
                        .await
                        .map(|outcome| outcome.map(Box::new))
                }
                None => Err("Analysis services unavailable".into()),
            };
            cx.update(|editor| {
                editor.analysis_result(id, result, &cx);
                editor.acquire_working(&cx);
            });
        }) else {
            self.analysis_result(id, Err("Application stopped".into()), cx);

            return;
        };
        self.document
            .analysis_pending
            .as_mut()
            .expect("live analysis fitting")
            .task = Some(task);
    }

    pub(super) fn analysis_progress(&mut self, id: u64, done: usize, total: usize) {
        let Some(pending) = self
            .document
            .analysis_pending
            .as_ref()
            .or(self.document.analysis_preparing.as_ref())
        else {
            return;
        };
        if pending.id != id {
            return;
        }
        self.tasks.report(
            TaskRequest::Analyze(id),
            TaskProgress::Counted {
                completed: done,
                total,
            },
        );
        if !pending.cancelled.is_cancelled() {
            self.status = if done < total {
                format!("Analyzing frame {} of {total}…", done + 1)
            } else {
                "Fitting gamma/WB/exposures and validating proposal…".into()
            };
        }
    }
}
