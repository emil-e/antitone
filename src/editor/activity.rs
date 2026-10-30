use super::{
    Editor,
    presentation::{Progress, ProgressContent},
};
use std::time::{Duration, Instant};

const PROMINENCE_DELAY: Duration = Duration::from_millis(300);

/// Foreground task presentation keyed by controller-owned requests.
#[derive(Clone, Copy, Debug)]
pub(super) enum Task {
    Import(u64),
    Open(u64),
    New(u64),
    Save(u64),
    Correction(u64),
    Input(u64),
    History(u64),
    Photo(u64),
    Reference(u64),
    Measurement(u64, &'static str),
    Analyze(u64),
    AnalysisAreas(u64),
    Regions(u64),
    Export(u64),
    Look(u64),
}

/// Scopes IDs that different worker operations may reuse; stale work cannot end another task.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum TaskRequest {
    Document(u64),
    Photo(u64),
    Reference(u64),
    Measurement(u64),
    Analyze(u64),
    Regions(u64),
    Export(u64),
    Look(u64),
}

#[derive(Clone, Copy)]
pub(super) enum TaskProgress {
    Indeterminate,
    Counted { completed: usize, total: usize },
}

impl Task {
    pub fn request(self) -> TaskRequest {
        match self {
            Self::Import(id)
            | Self::Open(id)
            | Self::New(id)
            | Self::Save(id)
            | Self::Correction(id)
            | Self::Input(id)
            | Self::History(id) => TaskRequest::Document(id),
            Self::Photo(id) => TaskRequest::Photo(id),
            Self::Reference(id) => TaskRequest::Reference(id),
            Self::Measurement(id, _) => TaskRequest::Measurement(id),
            Self::Analyze(id) | Self::AnalysisAreas(id) => TaskRequest::Analyze(id),
            Self::Regions(id) => TaskRequest::Regions(id),
            Self::Export(id) => TaskRequest::Export(id),
            Self::Look(id) => TaskRequest::Look(id),
        }
    }

    fn counts(self) -> bool {
        matches!(
            self,
            Self::Import(_)
                | Self::Open(_)
                | Self::Analyze(_)
                | Self::AnalysisAreas(_)
                | Self::Export(_)
                | Self::Regions(_)
        )
    }

    fn modal(self) -> bool {
        matches!(
            self.request(),
            TaskRequest::Document(_)
                | TaskRequest::Export(_)
                | TaskRequest::Analyze(_)
                | TaskRequest::Regions(_)
        )
    }

    fn cancel_action(self) -> Option<TaskCancel> {
        match self {
            Self::Analyze(_) | Self::AnalysisAreas(_) => Some(TaskCancel {
                request: self.request(),
                label: "Cancel analysis",
            }),
            Self::Export(_) => Some(TaskCancel {
                request: self.request(),
                label: "Cancel export",
            }),
            Self::Regions(_) => Some(TaskCancel {
                request: self.request(),
                label: "Cancel region preparation",
            }),
            Self::Look(_) => Some(TaskCancel {
                request: self.request(),
                label: "Cancel look preparation",
            }),
            _ => None,
        }
    }

    fn title(self) -> String {
        match self {
            Self::Import(_) => "Importing RAWs…".into(),
            Self::Open(_) => "Opening project…".into(),
            Self::New(_) => "Creating project…".into(),
            Self::Save(_) => "Saving project…".into(),
            Self::Correction(_) => "Applying correction…".into(),
            Self::Input(_) => "Switching input stage…".into(),
            Self::History(_) => "Restoring edit…".into(),
            Self::Photo(_) => "Loading photo…".into(),
            Self::Reference(_) => "Preparing reference…".into(),
            Self::Analyze(_) => "Analyzing roll…".into(),
            Self::AnalysisAreas(_) => "Preparing analysis areas…".into(),
            Self::Regions(_) => "Preparing film and picture regions…".into(),
            Self::Export(_) => "Exporting…".into(),
            Self::Look(_) => "Preparing look…".into(),
            Self::Measurement(_, label) => format!("Measuring {label}…"),
        }
    }
}

enum TaskState {
    Running(TaskProgress),
    Cancelling,
}

struct ActiveTask {
    task: Task,
    started: Instant,
    state: TaskState,
}

/// One foreground indicator. Controllers own dispatch, cancellation, snapshots and promotion.
#[derive(Default)]
pub(super) struct TaskActivity {
    active: Option<ActiveTask>,
}

#[derive(Clone, Copy)]
pub(super) struct TaskCancel {
    pub request: TaskRequest,
    pub label: &'static str,
}

pub(super) struct TaskView {
    pub title: String,
    pub measured: Option<(f32, String)>,
    pub cancel: Option<TaskCancel>,
    pub(super) modal: bool,
}

impl TaskView {
    fn presentation(&self) -> ProgressContent<'_> {
        ProgressContent {
            title: &self.title,
            progress: self.measured.as_ref().map_or(
                Progress::Indeterminate,
                |(fraction, label)| Progress::Measured {
                    fraction: *fraction,
                    label,
                },
            ),
            cancel: self.cancel.map(|action| action.label),
        }
    }
}

impl TaskView {
    pub fn compact(&self, ui: &mut egui::Ui) -> Option<TaskRequest> {
        if self.modal {
            ui.add(egui::Label::new(&self.title).truncate());
            return None;
        }
        if self.presentation().compact(ui) {
            self.cancel.map(|action| action.request)
        } else {
            None
        }
    }

    pub fn viewer(&self, ui: &mut egui::Ui, canvas: egui::Rect) {
        self.presentation().viewer(ui, canvas);
    }
}

impl TaskActivity {
    /// Replaces only presentation. The controller must supersede execution before beginning work.
    pub fn begin(&mut self, task: Task) {
        log::info!(task:?; "activity_started");

        self.active = Some(ActiveTask {
            task,
            started: Instant::now(),
            state: TaskState::Running(TaskProgress::Indeterminate),
        });
    }

    pub fn report(&mut self, request: TaskRequest, progress: TaskProgress) {
        let Some(active) = &mut self.active else {
            return;
        };
        if active.task.request() != request || matches!(active.state, TaskState::Cancelling) {
            return;
        }
        let progress = match progress {
            TaskProgress::Counted { .. } if !active.task.counts() => return,
            // The counted phase finishing does not finish validation/preparation or the task.
            TaskProgress::Counted { completed, total } if completed >= total => {
                TaskProgress::Indeterminate
            }
            progress => progress,
        };
        active.state = TaskState::Running(progress);
    }

    /// Call after signalling real cooperative cancellation; retain activity until terminal reply.
    pub fn cancelling(&mut self, request: TaskRequest) {
        if let Some(active) = &mut self.active
            && active.task.request() == request
            && active.task.cancel_action().is_some()
        {
            active.state = TaskState::Cancelling;
        }
    }

    pub fn is_current(&self, request: TaskRequest) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.task.request() == request)
    }

    /// Retire matching presentation; report whether it was still the foreground activity.
    /// Controllers retain execution ownership and decide what completion may change.
    pub fn finish(&mut self, request: TaskRequest) -> bool {
        let current = self.is_current(request);
        if current && let Some(active) = self.active.take() {
            log::info!(
                request:?,
                seconds = active.started.elapsed().as_secs_f64();
                "activity_finished"
            );
        }
        current
    }

    pub fn view(&self) -> Option<TaskView> {
        let active = self.active.as_ref()?;
        let (title, measured, cancel) = match active.state {
            TaskState::Cancelling => (
                match active.task {
                    Task::Look(_) => "Cancelling look preparation…",
                    Task::Export(_) => "Cancelling export…",
                    _ => "Cancelling analysis…",
                }
                .into(),
                None,
                None,
            ),
            TaskState::Running(progress) => {
                let measured = match progress {
                    TaskProgress::Indeterminate => None,
                    TaskProgress::Counted { completed, total } => Some((
                        completed as f32 / total as f32,
                        format!("{completed} of {total}"),
                    )),
                };
                let cancel = active.task.cancel_action();
                (active.task.title(), measured, cancel)
            }
        };
        Some(TaskView {
            title,
            measured,
            cancel,
            modal: active.task.modal(),
        })
    }

    pub fn blocks_workspace(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.task.modal())
    }

    pub fn modal(&self, context: &egui::Context, id: egui::Id) -> Option<TaskRequest> {
        let active = self.active.as_ref().filter(|active| active.task.modal())?;
        let remaining = PROMINENCE_DELAY.saturating_sub(active.started.elapsed());
        if !remaining.is_zero() {
            context.request_repaint_after(remaining);
            return None;
        }
        let view = self.view().expect("active task has presentation");
        if view.presentation().modal(context, id) {
            view.cancel.map(|action| action.request)
        } else {
            None
        }
    }

    #[cfg(test)]
    pub fn elapse(&mut self, duration: Duration) {
        if let Some(active) = &mut self.active {
            active.started -= duration;
        }
    }
}

impl Editor {
    pub(super) fn cancel_task(&mut self, request: TaskRequest) {
        if let TaskRequest::Export(id) = request {
            self.cancel_export(id);
            return;
        }
        if let TaskRequest::Regions(id) = request {
            if self
                .document
                .spatial_preparing
                .as_ref()
                .is_some_and(|p| p.id == id)
            {
                self.cancel_spatial();
            }
            return;
        }
        if let TaskRequest::Look(id) = request {
            if self
                .document
                .look
                .as_ref()
                .is_some_and(|p| p.activity == id)
            {
                self.cancel_look_selection();
            }
            return;
        }
        if let TaskRequest::Analyze(id) = request
            && self.analysis_work_id() == Some(id)
        {
            self.cancel_analysis();
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod modal_tests;
