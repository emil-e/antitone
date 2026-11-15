use super::*;
use crate::application::test_support::TestApplication;
use std::{collections::VecDeque, sync::Mutex, time::Duration};

pub(in crate::editor) struct Call {
    pub valid: bool,
    pub cancelled: crate::cancellation::Cancellation,
    release: std::sync::mpsc::Sender<()>,
}

impl Call {
    pub fn finish(self) {
        self.release.send(()).expect("preparation gate timed out");
    }
}

#[derive(Default)]
struct Waiting {
    calls: Mutex<VecDeque<Call>>,
    entered: event_listener::Event,
}

#[derive(Clone, Default)]
pub(in crate::editor) struct Gate(Arc<Waiting>);

impl Gate {
    pub fn install(editor: &mut Editor) -> Self {
        let gate = Self::default();
        let waiting = gate.0.clone();
        editor.look.services.as_mut().unwrap().prepared =
            Some(Arc::new(move |valid, cancelled| {
                let (release, receive) = std::sync::mpsc::channel();
                waiting.calls.lock().unwrap().push_back(Call {
                    valid,
                    cancelled,
                    release,
                });
                waiting.entered.notify(usize::MAX);
                match receive.recv_timeout(Duration::from_secs(60)) {
                    Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        panic!("preparation gate was not released")
                    }
                }
            }));
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
        app.run_until(Duration::from_secs(15), |_| {
            !self.0.calls.lock().unwrap().is_empty()
        });
        drop(notification);
        self.0.calls.lock().unwrap().pop_front().unwrap()
    }
}
