use crate::{
    application::test_support::TestApplication, cancellation::Cancellation, editor::Editor,
};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

#[derive(Default)]
struct Waiting {
    calls: Mutex<VecDeque<Call>>,
    entered: event_listener::Event,
}

#[derive(Clone, Default)]
pub(in crate::editor) struct Gate(Arc<Waiting>, bool);

#[derive(Clone, Default)]
pub(in crate::editor) struct Completion(Arc<Completed>);

#[derive(Default)]
struct Completed {
    done: AtomicBool,
    changed: event_listener::Event,
}

struct Acknowledge(Completion);
impl Drop for Acknowledge {
    fn drop(&mut self) {
        self.0.0.done.store(true, Ordering::Release);
        self.0.0.changed.notify(usize::MAX);
    }
}

impl Completion {
    pub fn wait(&self, app: &mut TestApplication<'_, Editor>) {
        let completed = self.clone();
        let task = app
            .access()
            .spawn(move |_| async move {
                let changed = completed.0.changed.listen();
                if !completed.0.done.load(Ordering::Acquire) {
                    changed.await;
                }
            })
            .unwrap();
        app.run_until(Duration::from_secs(5), |_| task.is_finished());
    }
}

pub(in crate::editor) struct Call {
    pub cancelled: Cancellation,
    release: mpsc::Sender<bool>,
    pub completed: Completion,
}

impl Call {
    pub fn run(self) {
        let _ = self.release.send(false);
    }

    pub fn panic(self) {
        let _ = self.release.send(true);
    }
}

impl Gate {
    pub fn after_work() -> Self {
        Self(Arc::default(), true)
    }

    pub fn is_after_work(&self) -> bool {
        self.1
    }

    pub fn install(editor: &mut Editor) -> Self {
        let gate = Self::default();
        editor.document.visit.measurement_gate = Some(gate.clone());
        gate
    }

    pub fn next(&self, app: &mut TestApplication<'_, Editor>) -> Call {
        let waiting = self.0.clone();
        let notification = app
            .access()
            .spawn(move |_| async move {
                let entered = waiting.entered.listen();
                if waiting.calls.lock().unwrap().is_empty() {
                    entered.await;
                }
            })
            .unwrap();
        app.run_until(Duration::from_secs(5), |_| {
            !self.0.calls.lock().unwrap().is_empty()
        });
        drop(notification);
        self.0.calls.lock().unwrap().pop_front().unwrap()
    }

    pub(in crate::editor) fn wait(&self, cancelled: Cancellation) -> Result<(), String> {
        let (release, resume) = mpsc::channel();
        let completed = Completion::default();
        let _acknowledge = Acknowledge(completed.clone());
        self.0.calls.lock().unwrap().push_back(Call {
            cancelled: cancelled.clone(),
            release,
            completed,
        });
        self.0.entered.notify(usize::MAX);
        let panic = resume
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| "measurement test gate dropped or timed out".to_owned())?;
        if !self.is_after_work() && cancelled.is_cancelled() {
            return Err("Measurement cancelled".into());
        }
        assert!(!panic, "injected measurement worker panic");
        Ok(())
    }
}
