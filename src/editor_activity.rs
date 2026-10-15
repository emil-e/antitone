use super::{
    Editor, Pick,
    presentation::{Progress, ProgressContent},
};
use std::time::{Duration, Instant};

const PROMINENCE_DELAY: Duration = Duration::from_millis(300);

/// Foreground work with its existing controller request identity. This is not an executor.
#[derive(Clone, Copy)]
pub(super) enum Task {
    Import(u64),
    Open(u64),
    New(u64),
    Save(u64),
    Correction(u64),
    Photo(u64),
    Reference(u64),
    Measurement(u64, Pick),
    Analyze(u64),
    AnalysisAreas(u64),
    Export(u64),
}

/// Scopes IDs that different worker operations may reuse; stale work cannot end another task.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum TaskRequest {
    Document(u64),
    Photo(u64),
    Reference(u64),
    Measurement(u64, Pick),
    Analyze(u64),
    Export(u64),
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
            | Self::Correction(id) => TaskRequest::Document(id),
            Self::Photo(id) => TaskRequest::Photo(id),
            Self::Reference(id) => TaskRequest::Reference(id),
            Self::Measurement(id, pick) => TaskRequest::Measurement(id, pick),
            Self::Analyze(id) | Self::AnalysisAreas(id) => TaskRequest::Analyze(id),
            Self::Export(id) => TaskRequest::Export(id),
        }
    }

    fn counts(self) -> bool {
        matches!(
            self,
            Self::Import(_) | Self::Open(_) | Self::Analyze(_) | Self::AnalysisAreas(_)
        )
    }
    fn modal(self) -> bool {
        matches!(self.request(), TaskRequest::Document(_))
    }

    fn cancel_action(self) -> Option<TaskCancel> {
        matches!(self, Self::Analyze(_) | Self::AnalysisAreas(_)).then_some(TaskCancel {
            request: self.request(),
            label: "Cancel analysis",
        })
    }

    fn title(self) -> String {
        match self {
            Self::Import(_) => "Importing RAWs…".into(),
            Self::Open(_) => "Opening project…".into(),
            Self::New(_) => "Creating project…".into(),
            Self::Save(_) => "Saving project…".into(),
            Self::Correction(_) => "Applying correction…".into(),
            Self::Photo(_) => "Loading photo…".into(),
            Self::Reference(_) => "Preparing reference…".into(),
            Self::Analyze(_) => "Analyzing roll…".into(),
            Self::AnalysisAreas(_) => "Preparing analysis areas…".into(),
            Self::Export(_) => "Exporting…".into(),
            Self::Measurement(_, pick) => {
                let name = match pick {
                    Pick::Dmin => "Dmin",
                    Pick::Dmax => "Dmax",
                    Pick::Gamma => "Gamma",
                    Pick::RollWb => "Roll WB",
                    Pick::Wb => "WB",
                    Pick::Flat | Pick::Analysis => {
                        unreachable!("area tools do not dispatch measurements")
                    }
                };
                format!("Measuring {name}…")
            }
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
        if self.presentation().compact(ui) {
            self.cancel.map(|action| action.request)
        } else {
            None
        }
    }

    pub fn inline(&self, ui: &mut egui::Ui) {
        let mut content = self.presentation();
        // Cancellation belongs to the global activity strip, independent of the selected stage.
        content.cancel = None;
        content.inline(ui);
    }

    pub fn viewer(&self, ui: &mut egui::Ui, canvas: egui::Rect) {
        self.presentation().viewer(ui, canvas);
    }
}

impl TaskActivity {
    /// Replaces only presentation. The controller must supersede execution before beginning work.
    pub fn begin(&mut self, task: Task) {
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

    /// Retire matching presentation; report whether it was still the foreground activity.
    /// Controllers retain execution ownership and decide what completion may change.
    pub fn finish(&mut self, request: TaskRequest) -> bool {
        let current = self
            .active
            .as_ref()
            .is_some_and(|active| active.task.request() == request);
        if current {
            self.active = None;
        }
        current
    }

    pub fn view(&self) -> Option<TaskView> {
        let active = self.active.as_ref()?;
        let (title, measured, cancel) = match active.state {
            TaskState::Cancelling => ("Cancelling analysis…".into(), None, None),
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
        })
    }

    pub fn modal(&self, context: &egui::Context, id: egui::Id) {
        if self.active.as_ref().is_some_and(|active| {
            active.task.modal() && active.started.elapsed() >= PROMINENCE_DELAY
        }) {
            self.view()
                .expect("active task has presentation")
                .presentation()
                .modal(context, id);
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
        if let TaskRequest::Analyze(id) = request
            && self
                .analysis_pending
                .as_ref()
                .or(self.analysis_preparing.as_ref())
                .is_some_and(|pending| pending.id == id)
        {
            self.cancel_analysis();
        }
    }
}

#[cfg(test)]
#[path = "editor_activity_tests.rs"]
mod tests;
