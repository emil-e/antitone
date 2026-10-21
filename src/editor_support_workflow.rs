#[cfg(test)]
#[path = "editor_support_workflow_tests.rs"]
mod tests;

use super::{Editor, source};
use crate::{blocking_jobs::BlockingJobs, raw::Rect};

#[derive(Default)]
pub(super) struct State {
    pub pending: Option<Pending>,
    task: Option<async_executor::Task<()>>,
}

pub(super) struct Pending {
    pub id: u64,
    pub loaded: u64,
    pub input: Option<source::Context>,
}

pub(super) async fn prepare(
    jobs: BlockingJobs,
    input: source::Context,
) -> Result<Option<Rect>, String> {
    jobs.run(move || input.scan()?.propose_flat_field_area())
        .await
        .map_err(|_| "Support preparation worker panicked".to_owned())?
        .map_err(|error| error.to_string())
}

impl Editor {
    pub(super) fn start_support_workflow(
        &mut self,
        access: &crate::application::UiAccess<'_, Self>,
    ) {
        let Some(pending) = self.document.visit.support.pending.as_mut() else {
            return;
        };
        let Some(input) = pending.input.take() else {
            return;
        };
        let (request, loaded) = (pending.id, pending.loaded);
        let jobs = self.workflow_jobs.clone();
        let target = access.clone();
        let Some(task) = access.spawn(async move {
            let result = prepare(jobs, input).await;
            target.update(|editor| editor.analysis_support_result(request, loaded, result));
        }) else {
            self.analysis_support_result(request, loaded, Err("Application stopped".into()));
            return;
        };
        self.document.visit.support.task = Some(task);
    }
}
