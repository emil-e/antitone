#[cfg(test)]
mod tests;

use super::{
    Editor, OriginalPurpose, UiContext,
    original_view::{Demand, Request},
    source,
};
use std::sync::Arc;

impl Request {
    pub async fn prepare(
        self,
        services: source::Services,
        submissions: crate::platform::SubmissionGate,
    ) -> Result<Option<Arc<crate::viewer::GpuImage>>, String> {
        let demand = self.demand;
        crate::background::run(move |stop| {
            if stop.is_cancelled() {
                return Ok(None);
            }
            let source = &demand.source;
            let scan = source.scan()?;
            let renderer = services.renderers.get(&services.device, &services.queue)?;
            let original = demand.capture.original(
                &services.device,
                &renderer,
                &scan,
                source
                    .correction
                    .as_ref()
                    .map(|correction| correction.gpu.clone()),
                &demand.repairs,
                demand.target,
            )?;
            crate::refinement::refine_definition(
                &renderer,
                &services.device,
                (&services.queue, &submissions),
                original,
                (demand.target, demand.view.expect("requested original view")),
                demand.capture.look_id().is_some(),
                || stop.is_cancelled(),
            )
        })
        .await
        .map_err(|_| "Original reconstruction worker panicked".to_owned())?
        .map_err(|error: crate::Error| error.to_string())
    }
}

impl Editor {
    pub(super) fn invalidate_original(&mut self) {
        self.document.visit.canvas.grade.original.invalidate();
    }
    pub(super) fn discard_original(&mut self) {
        self.document.visit.canvas.grade.original.discard();
    }
    pub(super) fn cancel_original_pending(&mut self) {
        self.document.visit.canvas.grade.original.cancel_pending();
    }
    pub(super) fn leave_original(&mut self) {
        self.active_canvas_mut().grade.original.leave();
    }

    pub(super) fn observe_original(
        &mut self,
        purpose: Option<OriginalPurpose>,
        demand: Option<Demand>,
    ) {
        match (purpose, demand) {
            (Some(purpose), Some(demand)) => self
                .active_canvas_mut()
                .grade
                .original
                .observe(purpose, demand),
            _ => self.leave_original(),
        }
    }

    pub(super) fn request_original(&mut self, context: &egui::Context, cx: &UiContext<'_, Self>) {
        if let Some(request) = self.active_canvas().grade.original.request() {
            let Some(completion) = request.begin() else {
                return;
            };
            let Some(services) = self.source_services.clone() else {
                if let Some(error) =
                    completion.receive(Err("Original reconstruction services unavailable".into()))
                {
                    self.status = error;
                }
                return;
            };
            let submissions = self.submissions.clone();
            let completion_on_shutdown = completion.clone();
            let Some(task) = cx.spawn(move |cx| async move {
                let result = request.prepare(services, submissions).await;
                cx.update(|editor| {
                    let visible = completion.belongs_to(&editor.active_canvas().grade.original);
                    if let Some(error) = completion.receive(result)
                        && visible
                    {
                        editor.status = error;
                    }
                });
            }) else {
                if let Some(error) =
                    completion_on_shutdown.receive(Err("Application stopped".into()))
                {
                    self.status = error;
                }
                return;
            };
            self.active_canvas_mut().grade.original.task = Some(task);
        }
        let original = &self.active_canvas().grade.original;
        if original.refining() || original.settled().is_none() && !original.failed() {
            context.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
}
