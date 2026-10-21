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
    Editor, Error, TaskProgress, TaskRequest, analysis, analysis_review,
    analysis_services::{Cancellation, Services},
    source,
};
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    future::Future,
    sync::{Arc, Mutex},
};

pub(super) struct Pending {
    pub id: u64,
    pub request: analysis::Request,
    input: Option<source::Context>,
    cancelled: Cancellation,
    task: Option<async_executor::Task<()>>,
}

impl Pending {
    pub fn new(id: u64, request: analysis::Request) -> Self {
        Self {
            id,
            request,
            input: None,
            cancelled: Cancellation::default(),
            task: None,
        }
    }

    pub fn fitting(id: u64, request: analysis::Request, input: source::Context) -> Self {
        let mut pending = Self::new(id, request);
        pending.input = Some(input);
        pending
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

struct Retire(Cancellation);
impl Drop for Retire {
    fn drop(&mut self) {
        self.0.cancel();
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
    target: &crate::application::UiAccess<'_, Editor>,
    id: u64,
) -> Result<Option<T>, String> {
    let reports = async {
        loop {
            let changed = progress.changed.listen();
            let latest = progress.latest.lock().unwrap().take();
            if let Some((done, total)) = latest {
                target.update(|editor| editor.analysis_progress(id, done, total));
            }
            changed.await;
        }
    };
    futures_lite::future::or(work, reports)
        .await
        .map_err(|error| error.to_string())
}

impl Editor {
    pub(super) fn start_analysis_preparation(
        &mut self,
        access: &crate::application::UiAccess<'_, Self>,
    ) {
        let Some(pending) = &self.document.analysis_preparing else {
            return;
        };
        if pending.task.is_some() {
            return;
        }
        let id = pending.id;
        let request = pending.request.clone();
        let cancelled = pending.cancelled.clone();
        let Some(services) = self.source_services.as_ref().map(Services::from) else {
            self.analysis_areas_result(id, Err("Analysis services unavailable".into()));
            return;
        };
        let progress = Arc::new(Progress::default());
        let produced = progress.clone();
        let target = access.clone();
        let retire = Retire(cancelled.clone());
        let Some(task) = access.spawn(async move {
            let _retire = retire;
            let preparation = analysis_review::preparation::prepare(
                request,
                services,
                cancelled.clone(),
                move |done, total| produced.report(done, total),
            );
            let result = drive(preparation, progress, &target, id).await;
            target.update(|editor| editor.analysis_areas_result(id, result));
        }) else {
            self.analysis_areas_result(id, Err("Application stopped".into()));
            return;
        };
        self.document
            .analysis_preparing
            .as_mut()
            .expect("live area preparation")
            .task = Some(task);
    }

    pub(super) fn start_analysis_workflow(
        &mut self,
        access: &crate::application::UiAccess<'_, Self>,
    ) {
        let Some(pending) = &mut self.document.analysis_pending else {
            return;
        };
        let Some(input) = pending.input.take() else {
            return;
        };
        let id = pending.id;
        let request = pending.request.clone();
        let cancelled = pending.cancelled.clone();
        let Some(services) = self.source_services.as_ref().map(Services::from) else {
            self.analysis_result(id, Err("Analysis services unavailable".into()));
            self.start_source_workflow(access);
            return;
        };
        let progress = Arc::new(Progress::default());
        let produced = progress.clone();
        let target = access.clone();
        let retire = Retire(cancelled.clone());
        let Some(task) = access.spawn(async move {
            let _retire = retire;
            let fitting = analysis::fit::run(
                request,
                input,
                services,
                cancelled.clone(),
                move |done, total| produced.report(done, total),
            );
            let result = drive(fitting, progress, &target, id)
                .await
                .map(|outcome| outcome.map(Box::new));
            target.update(|editor| {
                editor.analysis_result(id, result);
                editor.start_source_workflow(&target);
                editor.start_working_workflow(&target);
                editor.start_original_workflow(&target);
            });
        }) else {
            self.analysis_result(id, Err("Application stopped".into()));
            self.start_source_workflow(access);
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
