#[cfg(test)]
#[path = "editor_support_workflow_tests.rs"]
mod tests;

use super::{Coverage, Editor, UiContext, source};
use crate::{blocking_jobs::BlockingJobs, raw::Rect};

#[derive(Default)]
pub(super) struct State {
    pub pending: Option<Pending>,
    task: Option<async_executor::Task<()>>,
}

pub(super) struct Pending {
    pub id: u64,
    pub loaded: u64,
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
    pub(super) fn request_analysis_support(&mut self, cx: &UiContext<'_, Self>) {
        self.region = self.analysis_picture().map(Coverage::rectangle);
        if self.region.is_some()
            || self
                .document
                .visit
                .ready
                .as_ref()
                .is_none_or(|ready| ready.analysis_support.is_some())
        {
            return;
        }
        let input = self.source_context().expect("ready support source");
        let request = self.request_id();
        let loaded = self.id;
        self.document.visit.support.pending = Some(Pending {
            id: request,
            loaded,
        });
        let jobs = self.workflow_jobs.clone();
        let Some(task) = cx.spawn(move |cx| async move {
            let result = prepare(jobs, input).await;
            cx.update(|editor| editor.analysis_support_result(request, loaded, result));
        }) else {
            self.analysis_support_result(request, loaded, Err("Application stopped".into()));
            return;
        };
        self.document.visit.support.task = Some(task);
    }
}
