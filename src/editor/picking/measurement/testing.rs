use crate::{
    application::test_support::TestApplication, cancellation::Cancellation, editor::Editor,
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

#[derive(Default)]
struct Waiting {
    calls: Mutex<VecDeque<Call>>,
    entered: event_listener::Event,
}

#[derive(Clone, Default)]
pub(in crate::editor) struct Gate(Arc<Waiting>);

pub(in crate::editor) struct Call {
    pub cancelled: Cancellation,
    release: mpsc::Sender<bool>,
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
    pub fn install(editor: &mut Editor) -> Self {
        let gate = Self::default();
        editor.document.visit.picking.measurement_gate = Some(gate.clone());
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

    pub(super) fn wait(&self, cancelled: Cancellation) -> Result<(), String> {
        let (release, resume) = mpsc::channel();
        self.0.calls.lock().unwrap().push_back(Call {
            cancelled: cancelled.clone(),
            release,
        });
        self.0.entered.notify(usize::MAX);
        let panic = resume
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| "measurement test gate dropped or timed out".to_owned())?;
        if cancelled.is_cancelled() {
            return Err("Measurement cancelled".into());
        }
        assert!(!panic, "injected measurement worker panic");
        Ok(())
    }
}
