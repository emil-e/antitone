use super::*;
use crate::{application::test_support::TestApplication, editor::Editor, geometry::SourceRegion};
use std::{
    collections::VecDeque,
    sync::{Mutex, mpsc},
    time::Duration,
};

type Outcome = Result<SearchOutcome, String>;

pub(in crate::editor::dust) struct Call {
    pub input: search::Input,
    pub cancelled: Cancellation,
    result: mpsc::Sender<Outcome>,
}

impl Call {
    pub fn complete(self, result: Outcome) {
        let _ = self.result.send(result);
    }
}

#[derive(Default)]
struct Waiting {
    calls: Mutex<VecDeque<Call>>,
    entered: event_listener::Event,
}

#[derive(Clone, Default)]
pub(in crate::editor::dust) struct Gate(Arc<Waiting>);

impl Gate {
    pub fn install(editor: &mut Editor) -> Self {
        let gate = Self::default();
        editor.document.dust.search_worker = Some(Search::Gated(gate.clone()));
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

    pub fn is_empty(&self) -> bool {
        self.0.calls.lock().unwrap().is_empty()
    }
}

#[derive(Clone)]
pub(in crate::editor::dust) enum Search {
    Gated(Gate),
    Geometry(SourceRegion),
}

impl Search {
    pub(in crate::editor::dust) fn run(
        &self,
        input: search::Input,
        cancelled: Cancellation,
    ) -> Outcome {
        match self {
            Self::Gated(gate) => {
                let (result, receive) = mpsc::channel();
                gate.0.calls.lock().unwrap().push_back(Call {
                    input,
                    cancelled,
                    result,
                });
                gate.0.entered.notify(usize::MAX);
                receive
                    .recv_timeout(Duration::from_secs(10))
                    .map_err(|_| "test search gate dropped or timed out".to_owned())?
            }
            Self::Geometry(active) => crate::dust_repair::search_donor(
                &input.support,
                input.previous_offset,
                *active,
                &mut |_| panic!("synthetic geometry must not read pixels"),
                std::time::Instant::now(),
                0.,
                &mut crate::dust_repair::Execution::default(),
            )
            .map_err(|error| error.to_string()),
        }
    }
}
