#[cfg(test)]
mod acknowledgement_tests;
#[cfg(test)]
mod fit_tests;
#[cfg(test)]
mod operation_tests;
#[cfg(test)]
pub(super) mod testing;
#[cfg(test)]
pub(super) mod tests;

use super::{
    Editor, Error, Task, TaskProgress, TaskRequest, UiContext, analysis, analysis_review,
    analysis_services::{Cancellation, Services},
    source,
};
use futures_channel::oneshot;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    future::Future,
    sync::{Arc, Mutex},
};

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Phase {
    Regions,
    Preparing(u64),
    Review,
    Fitting(u64),
}

pub(super) struct Operation {
    pub phase: Phase,
    pub cancelled: Cancellation,
    pub decision: Option<oneshot::Sender<Confirmation>>,
    pub task: Option<async_executor::Task<()>>,
}

impl Operation {
    pub(super) fn new(phase: Phase) -> Self {
        Self {
            phase,
            cancelled: Cancellation::default(),
            decision: None,
            task: None,
        }
    }
}

impl Drop for Operation {
    fn drop(&mut self) {
        self.cancelled.cancel();
    }
}

struct FitInput {
    id: u64,
    source: source::Context,
    services: Option<Services>,
}

pub(super) struct Confirmation {
    selected: super::SourceId,
    areas: analysis_review::Approved,
    fit: FitInput,
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

async fn sampling_review(
    request: analysis::Request,
    id: u64,
    services: Option<Services>,
    cancelled: Cancellation,
    cx: UiContext<'_, Editor>,
) {
    let progress = Arc::new(Progress::default());
    let produced = progress.clone();
    let result = async {
        #[cfg(test)]
        if let Some(work) = cx
            .update(|editor| editor.document.analysis_preparation_work.take())
            .flatten()
        {
            return work(request.clone(), cancelled.clone()).await;
        }
        match services {
            Some(services) => {
                drive(
                    analysis_review::preparation::prepare(
                        request.clone(),
                        services,
                        cancelled.clone(),
                        move |done, total| produced.report(done, total),
                    ),
                    progress,
                    &cx,
                    id,
                )
                .await
            }
            None => Err("Analysis services unavailable".into()),
        }
    }
    .await;
    let decision = cx
        .update(|editor| {
            editor.finish_analysis_work();
            if cancelled.is_cancelled() {
                editor.status = "Analysis-area preparation cancelled; grade unchanged".into();
                return None;
            }
            editor.open_analysis_review(&request, result, &cx)
        })
        .flatten();
    await_review(request, decision, cancelled, &cx).await;
}

async fn await_review(
    mut request: analysis::Request,
    decision: Option<oneshot::Receiver<Confirmation>>,
    cancelled: Cancellation,
    cx: &UiContext<'_, Editor>,
) {
    if let Some(decision) = decision
        && let Ok(confirmation) = decision.await
    {
        request.selected = confirmation.selected;
        request.areas = Some(confirmation.areas);
        fit_and_accept(request, confirmation.fit, cancelled, cx).await;
        return;
    }
    cx.update(|editor| editor.document.set_analysis_operation(None));
}

async fn fit_and_accept(
    request: analysis::Request,
    fit: FitInput,
    cancelled: Cancellation,
    cx: &UiContext<'_, Editor>,
) {
    let FitInput {
        id,
        source: input,
        services,
    } = fit;
    let progress = Arc::new(Progress::default());
    let produced = progress.clone();
    let result = async {
        #[cfg(test)]
        if let Some(work) = cx
            .update(|editor| editor.document.analysis_fit_work.take())
            .flatten()
        {
            return work(request.clone(), cancelled.clone()).await;
        }
        match services {
            Some(services) => drive(
                analysis::fit::run(
                    request.clone(),
                    input,
                    services,
                    cancelled.clone(),
                    move |done, total| produced.report(done, total),
                ),
                progress,
                cx,
                id,
            )
            .await
            .map(|outcome| outcome.map(Box::new)),
            None => Err("Analysis services unavailable".into()),
        }
    }
    .await;
    cx.update(|editor| {
        editor.finish_analysis_work();
        editor.accept_analysis(&request, result, cancelled.is_cancelled(), cx);
        editor.acquire_working(cx);
        editor.document.set_analysis_operation(None);
    });
}

impl Editor {
    fn analysis_services(&self) -> Option<Services> {
        self.source_services.as_ref().map(|source| {
            Services::new(
                source,
                source.provider(Some(self.document.cache.clone())),
                self.document.exposures.clone(),
            )
        })
    }

    pub(super) fn analysis_preparing(&self) -> bool {
        self.document
            .analysis_operation()
            .is_some_and(|op| matches!(op.phase, Phase::Preparing(_)))
    }

    pub(super) fn analysis_fitting(&self) -> bool {
        self.document
            .analysis_operation()
            .is_some_and(|op| matches!(op.phase, Phase::Fitting(_)))
    }

    pub(super) fn analysis_work_id(&self) -> Option<u64> {
        match self.document.analysis_operation()?.phase {
            Phase::Preparing(id) | Phase::Fitting(id) => Some(id),
            Phase::Regions | Phase::Review => None,
        }
    }

    fn begin_analysis_preparation(&mut self) -> (u64, Option<Services>) {
        let id = self.request_id();
        self.document.analysis_operation_mut().unwrap().phase = Phase::Preparing(id);
        self.document.thumbnails.cancel();
        self.tasks.begin(Task::AnalysisAreas(id));
        self.busy = true;
        self.status = "Preparing per-photo analysis areas…".into();
        (id, self.analysis_services())
    }

    fn finish_analysis_work(&mut self) {
        if let Some(id) = self.analysis_work_id()
            && self.tasks.finish(TaskRequest::Analyze(id))
        {
            self.busy = self.document_pending.is_some();
        }
        if let Some(op) = self.document.analysis_operation_mut() {
            op.phase = Phase::Review;
        }
    }

    pub(super) fn request_analysis(&mut self, cx: &UiContext<'_, Self>) {
        if !self.prepare_tool() {
            return;
        }
        let Some(mut request) = self.analysis_request() else {
            return;
        };
        let Some(regions) = self.prepare_regions(cx) else {
            return;
        };
        let operation = Operation::new(Phase::Regions);
        let cancelled = operation.cancelled.clone();
        let retire = cancelled.cancel_on_drop();
        self.document.set_analysis_operation(Some(operation));
        let task = cx.spawn(move |cx| async move {
            let _retire = retire;
            if regions.await.is_none() {
                cx.update(|editor| editor.document.set_analysis_operation(None));
                return;
            }
            let prepared = cx
                .update(|editor| {
                    super::document::rebase_project(
                        &mut request.document.project,
                        request.document.path.as_deref(),
                        editor.document.path.as_deref(),
                    );
                    request.document = editor.document.capture();
                    Some(editor.begin_analysis_preparation())
                })
                .flatten();
            if let Some((id, services)) = prepared {
                sampling_review(request, id, services, cancelled, cx.clone()).await;
            } else {
                cx.update(|editor| editor.document.set_analysis_operation(None));
            }
        });
        if let Some(task) = task {
            self.document.analysis_operation_mut().unwrap().task = Some(task);
        } else {
            self.document.set_analysis_operation(None);
            self.cancel_spatial();
        }
    }

    fn begin_analysis_fit(&mut self, total: usize) -> Option<FitInput> {
        let input = self.source_context()?;
        let id = self.request_id();
        self.document.analysis_operation_mut().unwrap().phase = Phase::Fitting(id);
        self.document.thumbnails.cancel();
        self.cancel_original_pending();
        self.tasks.begin(Task::Analyze(id));
        self.tasks.report(
            TaskRequest::Analyze(id),
            TaskProgress::Counted {
                completed: 0,
                total,
            },
        );
        self.busy = true;
        self.status = "Analyzing roll…".into();
        Some(FitInput {
            id,
            source: input,
            services: self.analysis_services(),
        })
    }

    pub(super) fn submit_analysis_review(&mut self, areas: analysis_review::Approved) {
        if !self
            .document
            .analysis_operation()
            .is_some_and(|op| op.decision.is_some())
        {
            return;
        }
        let Some(fit) = self.begin_analysis_fit(self.document.project.order.len()) else {
            return;
        };
        let decision = self
            .document
            .analysis_operation_mut()
            .unwrap()
            .decision
            .take()
            .unwrap();
        let _ = decision.send(Confirmation {
            selected: self.document.visit.current_source.expect("review source"),
            areas,
            fit,
        });
    }

    pub(super) fn cancel_analysis(&mut self) {
        let Some(op) = &self.document.analysis_operation() else {
            return;
        };
        if op.phase == Phase::Regions {
            self.document.set_analysis_operation(None);
            self.cancel_spatial();
            return;
        }
        op.cancelled.cancel();
        if let Some(id) = self.analysis_work_id() {
            self.tasks.cancelling(TaskRequest::Analyze(id));
        }
        self.status = "Cancelling analysis…".into();
    }

    pub(super) fn analysis_progress(&mut self, id: u64, done: usize, total: usize) {
        if self.analysis_work_id() != Some(id) {
            return;
        }
        self.tasks.report(
            TaskRequest::Analyze(id),
            TaskProgress::Counted {
                completed: done,
                total,
            },
        );
        if !self
            .document
            .analysis_operation()
            .unwrap()
            .cancelled
            .is_cancelled()
        {
            self.status = if done < total {
                format!("Analyzing frame {} of {total}…", done + 1)
            } else {
                "Fitting gamma/WB/exposures and validating proposal…".into()
            };
        }
    }
}
