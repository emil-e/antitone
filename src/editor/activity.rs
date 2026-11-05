use super::presentation::{Progress, ProgressContent};
use crate::cancellation::Cancellation;
use std::{
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const PROMINENCE_DELAY: Duration = Duration::from_millis(300);

pub(super) struct Options {
    title: String,
    modal: bool,
    cancel: Option<Cancel>,
}

struct Cancel {
    label: &'static str,
    status: &'static str,
    signal: Cancellation,
}

impl Options {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            modal: false,
            cancel: None,
        }
    }

    pub fn modal(mut self) -> Self {
        self.modal = true;
        self
    }

    pub fn cancellable(
        mut self,
        label: &'static str,
        status: &'static str,
        signal: Cancellation,
    ) -> Self {
        self.cancel = Some(Cancel {
            label,
            status,
            signal,
        });
        self
    }
}

#[derive(Default)]
struct Registry {
    entries: Mutex<Vec<Weak<Entry>>>,
    context: Mutex<Option<egui::Context>>,
}

impl Registry {
    fn repaint(&self) {
        if let Some(context) = self.context.lock().unwrap().as_ref() {
            context.request_repaint();
        }
    }
}

struct Entry {
    live: AtomicBool,

    started: Mutex<Instant>,
    modal: bool,
    cancel: Option<Cancel>,
    display: Mutex<Display>,
    registry: Weak<Registry>,
}

struct Display {
    title: String,
    measured: Option<(f32, String)>,
}

impl Entry {
    fn repaint(&self) {
        if let Some(registry) = self.registry.upgrade() {
            registry.repaint();
        }
    }

    fn live(&self) -> bool {
        self.live.load(Ordering::Acquire)
    }

    fn cancelling(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|cancel| cancel.signal.is_cancelled())
    }
}

/// Observes live activities without owning their lifetime.
#[derive(Default)]
pub(super) struct Activities(Arc<Registry>);

/// Owns the activity's visible lifetime.
#[must_use = "keep the activity in its workflow scope"]
pub(super) struct Activity(Arc<Entry>);

/// Workers may report progress without retaining the activity after its owner retires.
#[derive(Clone, Default, Debug)]
pub(super) struct Reporter(Weak<Entry>);

#[cfg(test)]
impl PartialEq for Reporter {
    fn eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }
}

impl Activities {
    pub fn set_context(&self, context: &egui::Context) {
        *self.0.context.lock().unwrap() = Some(context.clone());
    }

    pub fn start(&self, options: Options) -> Activity {
        let title = options.title;
        let entry = Arc::new(Entry {
            live: AtomicBool::new(true),

            started: Mutex::new(Instant::now()),
            display: Mutex::new(Display {
                title: String::new(),
                measured: None,
            }),
            modal: options.modal,
            cancel: options.cancel,
            registry: Arc::downgrade(&self.0),
        });
        let mut entries = self.0.entries.lock().unwrap();
        entries.retain(|entry| entry.upgrade().is_some_and(|entry| entry.live()));
        entries.push(Arc::downgrade(&entry));
        drop(entries);
        let activity = Activity(entry);
        activity.status(title);
        self.0.repaint();
        activity
    }

    pub fn view(&self) -> Option<View> {
        // Modal work takes precedence over local feedback; otherwise show the newest activity.
        let entries = self.0.entries.lock().unwrap();
        let mut selected = None;
        for entry in entries.iter().filter_map(Weak::upgrade) {
            if !entry.live() {
                continue;
            }
            if selected.as_ref().is_some_and(|view: &View| view.modal) && !entry.modal {
                continue;
            }
            selected = Reporter(Arc::downgrade(&entry)).view();
        }
        selected
    }

    #[cfg(test)]
    pub fn any(&self, mut matches: impl FnMut(&View) -> bool) -> bool {
        self.0.entries.lock().unwrap().iter().any(|entry| {
            Reporter(entry.clone())
                .view()
                .is_some_and(|view| matches(&view))
        })
    }

    pub fn blocks_workspace(&self) -> bool {
        self.0
            .entries
            .lock()
            .unwrap()
            .iter()
            .filter_map(Weak::upgrade)
            .any(|entry| entry.live() && entry.modal)
    }

    pub fn modal(&self, context: &egui::Context, id: egui::Id) {
        let Some(view) = self.view().filter(|view| view.modal) else {
            return;
        };
        let remaining = PROMINENCE_DELAY.saturating_sub(view.elapsed);
        if !remaining.is_zero() {
            context.request_repaint_after(remaining);
            return;
        }
        if view.presentation().modal(context, id) {
            view.request_cancel();
        }
    }

    #[cfg(test)]
    pub fn elapse(&self, duration: Duration) {
        for entry in self
            .0
            .entries
            .lock()
            .unwrap()
            .iter()
            .filter_map(Weak::upgrade)
        {
            *entry.started.lock().unwrap() -= duration;
        }
    }
}

impl Activity {
    pub fn reporter(&self) -> Reporter {
        Reporter(Arc::downgrade(&self.0))
    }
    pub fn status(&self, title: impl Into<String>) {
        self.reporter().status(title);
    }
    pub fn progress(&self, completed: usize, total: usize) {
        self.reporter().progress(completed, total);
    }

    pub async fn cancelled(&self) {
        match &self.0.cancel {
            Some(cancel) => cancel.signal.wait().await,
            None => std::future::pending().await,
        }
    }
}

impl Drop for Activity {
    fn drop(&mut self) {
        // Even a transient observer upgrade cannot extend the owning handle's lifetime.
        self.0.live.store(false, Ordering::Release);
        self.0.repaint();
    }
}

impl Reporter {
    fn entry(&self) -> Option<Arc<Entry>> {
        self.0.upgrade().filter(|entry| entry.live())
    }

    pub fn status(&self, title: impl Into<String>) {
        if let Some(entry) = self.entry().filter(|entry| !entry.cancelling()) {
            entry.display.lock().unwrap().title = title.into();
            entry.repaint();
        }
    }

    pub fn progress(&self, completed: usize, total: usize) {
        if let Some(entry) = self.entry().filter(|entry| !entry.cancelling()) {
            // A completed counted phase can still have validation/publication ahead of it.
            entry.display.lock().unwrap().measured = (completed < total).then(|| {
                (
                    completed as f32 / total as f32,
                    format!("{completed} of {total}"),
                )
            });
            entry.repaint();
        }
    }

    pub fn view(&self) -> Option<View> {
        let entry = self.entry()?;
        let display = entry.display.lock().unwrap();
        let cancelling = entry.cancelling();
        Some(View {
            title: if cancelling {
                entry.cancel.as_ref().unwrap().status.into()
            } else {
                display.title.clone()
            },
            measured: if cancelling {
                None
            } else {
                display.measured.clone()
            },
            cancel: entry
                .cancel
                .as_ref()
                .filter(|_| !cancelling)
                .map(|cancel| cancel.label),
            modal: entry.modal,
            elapsed: entry.started.lock().unwrap().elapsed(),
            reporter: self.clone(),
        })
    }

    pub fn request_cancel(&self) {
        if let Some(entry) = self.entry()
            && let Some(cancel) = &entry.cancel
        {
            cancel.signal.cancel();
            entry.repaint();
        }
    }
}

pub(super) struct View {
    pub title: String,
    pub measured: Option<(f32, String)>,
    pub cancel: Option<&'static str>,
    pub modal: bool,
    elapsed: Duration,
    reporter: Reporter,
}

impl View {
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
            cancel: self.cancel,
        }
    }

    pub fn request_cancel(&self) {
        self.reporter.request_cancel();
    }

    pub fn compact(&self, ui: &mut egui::Ui) {
        if self.modal {
            ui.add(egui::Label::new(&self.title).truncate());
        } else if self.presentation().compact(ui) {
            self.request_cancel();
        }
    }

    pub fn viewer(&self, ui: &mut egui::Ui, canvas: egui::Rect) {
        self.presentation().viewer(ui, canvas);
    }
}

#[cfg(test)]
mod lifecycle_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod modal_tests;
