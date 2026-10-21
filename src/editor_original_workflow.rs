#[cfg(test)]
#[path = "editor_original_workflow_tests.rs"]
mod tests;

use super::{Editor, original_view::Request, source};
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
        let jobs = services.jobs.clone();
        jobs.run(move || {
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
    pub(super) fn start_original_workflow(
        &mut self,
        access: &crate::application::UiAccess<'_, Self>,
    ) {
        if self
            .original
            .refining
            .is_none_or(|(running, _)| running != self.original.revision())
        {
            self.document.visit.grade.task = None;
            if let Some((retired, _)) = self.original.refining {
                self.original.receive(retired, Ok(None));
            }
        }
        let Some(request) = self.document.visit.grade.request.take() else {
            return;
        };
        let revision = request.revision;
        let generation = request.generation.clone();
        if self.original.revision() != revision {
            self.original.receive(revision, Ok(None));
            return;
        }
        let Some(services) = self.source_services.clone() else {
            self.original_result(
                revision,
                Err("Original reconstruction services unavailable".into()),
            );
            return;
        };
        let visit = self.document.visit.view;
        let submissions = self.submissions.clone();
        let retire = Retire {
            generation: generation.clone(),
            revision,
            armed: true,
        };
        let target = access.clone();
        let Some(task) = access.spawn(async move {
            let mut retire = retire;
            let result = request.prepare(services, submissions).await;
            retire.armed = false;
            target.update(|editor| {
                if editor.document.visit.view == visit
                    && Arc::ptr_eq(&editor.original.generation, &generation)
                {
                    editor.original_result(revision, result);
                }
            });
        }) else {
            self.original_result(revision, Err("Application stopped".into()));
            return;
        };
        self.document.visit.grade.task = Some(task);
    }

    pub(super) fn original_result(
        &mut self,
        revision: u64,
        result: Result<Option<Arc<crate::viewer::GpuImage>>, String>,
    ) {
        if self
            .original
            .refining
            .is_some_and(|(running, _)| running == revision)
        {
            self.document.visit.grade.task = None;
            if self
                .document
                .visit
                .grade
                .request
                .as_ref()
                .is_some_and(|request| request.revision == revision)
            {
                self.document.visit.grade.request = None;
            }
        }
        if let Some(error) = self.original.receive(revision, result) {
            self.status = error;
        }
    }
}
