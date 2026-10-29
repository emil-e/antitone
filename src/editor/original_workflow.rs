#[cfg(test)]
mod tests;

use super::{
    Editor, OriginalPurpose, OutputTarget, ProcessingInput, UiContext,
    original_view::{Request, Signature},
    source,
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

/// A departed continuation cancels its own revision, never a newer output demand.
struct Retire {
    generation: Arc<AtomicU64>,
    revision: u64,
    armed: bool,
}
impl Drop for Retire {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.generation.compare_exchange(
                self.revision,
                self.revision + 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
        }
    }
}

impl Request {
    pub async fn prepare(
        self,
        services: source::Services,
        submissions: crate::platform::SubmissionGate,
    ) -> Result<Option<Arc<crate::viewer::GpuImage>>, String> {
        crate::background::run(move |_| {
            if self.generation.load(Ordering::Relaxed) != self.revision {
                return Ok(None);
            }
            let scan = self.source.scan()?;
            let renderer = services.renderers.get(&services.device, &services.queue)?;
            crate::refinement::refine(
                &renderer,
                &services.device,
                (&services.queue, &submissions),
                (
                    &scan,
                    self.source.correction.as_ref().map(|c| c.gpu.clone()),
                ),
                self.input,
                self.target,
                (&self.generation, self.revision),
                self.view,
            )
        })
        .await
        .map_err(|_| "Original reconstruction worker panicked".to_owned())?
        .map_err(|error: crate::Error| error.to_string())
    }
}

impl Editor {
    pub(super) fn invalidate_original(&mut self) {
        let grade = &mut self.document.visit.canvas.grade;
        grade.original.invalidate();
        grade.task = None;
    }

    pub(super) fn discard_original(&mut self) {
        let grade = &mut self.document.visit.canvas.grade;
        grade.original.discard();
        grade.task = None;
    }

    pub(super) fn cancel_original_pending(&mut self) {
        let grade = &mut self.document.visit.canvas.grade;
        grade.original.cancel_pending();
        grade.task = None;
    }

    pub(super) fn leave_original(&mut self) {
        let grade = &mut self.active_canvas_mut().grade;
        grade.original.leave();
        grade.task = None;
    }

    pub(super) fn observe_original(
        &mut self,
        purpose: Option<OriginalPurpose>,
        signature: Signature,
    ) {
        match purpose {
            Some(purpose) => self
                .active_canvas_mut()
                .grade
                .original
                .observe(purpose, signature),
            None => self.leave_original(),
        }
        if self.active_canvas().grade.original.refining.is_none() {
            self.active_canvas_mut().grade.task = None;
        }
    }

    pub(super) fn request_original(
        &mut self,
        input: &ProcessingInput,
        target: OutputTarget,
        context: &egui::Context,
        source: source::Context,
        cx: &UiContext<'_, Self>,
    ) {
        if let Some(request) = self
            .active_canvas()
            .grade
            .original
            .request(input, target, source)
        {
            let revision = request.revision;
            let generation = request.generation.clone();
            self.active_canvas_mut().grade.original.refining = Some((revision, request.view));
            let Some(services) = self.source_services.clone() else {
                self.original_result(
                    revision,
                    Err("Original reconstruction services unavailable".into()),
                );
                return;
            };
            let submissions = self.submissions.clone();
            let retire = Retire {
                generation: generation.clone(),
                revision,
                armed: true,
            };
            let Some(task) = cx.spawn(move |cx| async move {
                let mut retire = retire;
                let result = request.prepare(services, submissions).await;
                retire.armed = false;
                cx.update(|editor| editor.receive_original_for(&generation, revision, result));
            }) else {
                self.original_result(revision, Err("Application stopped".into()));
                return;
            };
            self.active_canvas_mut().grade.task = Some(task);
        }
        let original = &self.active_canvas().grade.original;
        if original.refining.is_some()
            || original.settled.is_none() && original.refinement_error != Some(original.revision())
        {
            context.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }

    pub(super) fn original_result(
        &mut self,
        revision: u64,
        result: Result<Option<Arc<crate::viewer::GpuImage>>, String>,
    ) {
        let generation = self.active_canvas().grade.original.generation.clone();
        self.receive_original_for(&generation, revision, result);
    }

    fn receive_original_for(
        &mut self,
        generation: &Arc<AtomicU64>,
        revision: u64,
        result: Result<Option<Arc<crate::viewer::GpuImage>>, String>,
    ) {
        let visible = Arc::ptr_eq(generation, &self.active_canvas().grade.original.generation);
        let view = if Arc::ptr_eq(
            generation,
            &self.document.visit.canvas.grade.original.generation,
        ) {
            Some(&mut self.document.visit.canvas)
        } else {
            self.document
                .flat_edit
                .as_mut()
                .filter(|flat| Arc::ptr_eq(generation, &flat.view.grade.original.generation))
                .map(|flat| &mut flat.view)
        };
        let Some(view) = view else {
            return;
        };
        if view
            .grade
            .original
            .refining
            .is_some_and(|(running, _)| running == revision)
        {
            view.grade.task = None;
        }
        if let Some(error) = view.grade.original.receive(revision, result)
            && visible
        {
            self.status = error;
        }
    }
}
