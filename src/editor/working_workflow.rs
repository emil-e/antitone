use super::{
    Editor, source,
    working_view::{self, Acquisition, Completion},
};
use std::sync::Arc;

#[derive(Default)]
pub(super) struct State {
    pub task: Option<async_executor::Task<()>>,
}

impl Acquisition {
    pub async fn prepare(
        self,
        services: source::Services,
        submissions: crate::platform::SubmissionGate,
    ) -> Result<Option<Completion>, String> {
        let kernels = self.kernels.clone();
        let prepared = crate::background::run(move |stop| {
            if stop.is_cancelled() {
                return Ok(None);
            }
            let renderer = services.renderers.get(&services.device, &services.queue)?;
            let pipelines = if let Some(pipelines) = kernels.get() {
                pipelines.clone()
            } else {
                let prepared =
                    pollster::block_on(crate::gpu::view::ViewKernels::new(&services.device))?;
                kernels.get_or_init(|| prepared).clone()
            };
            Ok::<_, crate::Error>(Some((services, renderer, pipelines)))
        })
        .await
        .map_err(|_| "Working-source preparation worker panicked".to_owned())?
        .map_err(|error| error.to_string())?;
        let Some((services, renderer, kernels)) = prepared else {
            return Ok(None);
        };
        Ok(working_view::acquire_images(
            &self.mailbox,
            &renderer,
            &kernels,
            &services.device,
            &services.queue,
            &submissions,
        )
        .await)
    }
}

impl Editor {
    pub(super) fn request_working(
        &mut self,
        demand: working_view::Demand,
        cx: &crate::application::UiContext<'_, Self>,
    ) {
        self.document.visit.working_view.request(demand);
        self.acquire_working(cx);
    }

    pub(super) fn acquire_working(&mut self, cx: &crate::application::UiContext<'_, Self>) {
        if self.allow_close
            || self.document_pending.is_some()
            || self.document.visit.working.task.is_some()
        {
            return;
        }
        let Some(request) = self.document.visit.working_view.acquisition() else {
            return;
        };
        let Some(services) = self.source_services.clone() else {
            self.document
                .visit
                .working_view
                .acquisition_failed("Working-source services unavailable".into());
            return;
        };
        let owner = Arc::downgrade(&request.mailbox);
        let submissions = self.submissions.clone();
        let mailbox = request.mailbox.clone();
        let Some(task) = cx.spawn(move |cx| async move {
            let acquisition = request.prepare(services, submissions);
            let support = async {
                loop {
                    let changed = mailbox.1.listen();
                    cx.update(|editor| {
                        if editor.document.visit.working_view.owns(&owner) {
                            editor.document.visit.working_view.refresh_support();
                        }
                    });
                    changed.await;
                }
            };
            let result = futures_lite::future::or(acquisition, support).await;
            cx.update(|editor| {
                if !editor.document.visit.working_view.owns(&owner) {
                    return;
                }
                editor.document.visit.working.task = None;
                match result {
                    Ok(Some(completion)) => editor.document.visit.working_view.acquired(completion),
                    Ok(None) => {}
                    Err(error) => editor.document.visit.working_view.acquisition_failed(error),
                }
                editor.acquire_working(&cx);
            });
        }) else {
            self.document
                .visit
                .working_view
                .acquisition_failed("Application stopped".into());
            return;
        };
        self.document.visit.working.task = Some(task);
    }
}
