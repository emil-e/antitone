//! Purpose-bound original-sample reconstruction, independent of normal working-view acceptance.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(test)]
#[path = "editor_original_view_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OriginalPurpose {
    Reference,
    Tool,
}
pub(super) struct Request {
    pub revision: u64,
    pub generation: Arc<AtomicU64>,
    pub input: ProcessingInput,
    pub target: OutputTarget,
    pub view: ViewRequest,
    pub source: source::Context,
}

type Signature = (
    ProcessingSignature,
    OutputTarget,
    Option<ViewRequest>,
    Composition,
);

pub(super) struct OriginalView {
    pub generation: Arc<AtomicU64>,
    pub signature: Option<Signature>,
    purpose: Option<OriginalPurpose>,
    pub settled: Option<(u64, ViewRequest, Arc<GpuImage>)>,
    pub refining: Option<(u64, ViewRequest)>,
    pub last_edit: std::time::Instant,
    pub refinement_error: Option<u64>,
}

impl OriginalView {
    pub fn new(generation: Arc<AtomicU64>) -> Self {
        Self {
            generation,
            signature: None,
            purpose: None,
            settled: None,
            refining: None,
            last_edit: std::time::Instant::now(),
            refinement_error: None,
        }
    }

    pub fn revision(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    pub fn invalidate(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.signature = None;
        self.purpose = None;
        self.settled = None;
        self.refinement_error = None;
        self.refining = None;
        // Physical admission belongs to the running service, not this logical demand.
    }

    pub fn discard(&mut self) {
        if self.signature.is_some() || self.settled.is_some() {
            self.invalidate();
        } else {
            self.cancel_pending();
        }
    }

    pub fn cancel_pending(&mut self) {
        if self
            .refining
            .is_some_and(|(revision, _)| revision == self.revision())
        {
            let revision = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
            if let Some((accepted, _, _)) = &mut self.settled {
                *accepted = revision;
            }
        }
    }

    pub fn observe(&mut self, purpose: OriginalPurpose, signature: Signature) {
        if self.purpose != Some(purpose) || self.signature.as_ref() != Some(&signature) {
            self.invalidate();
            self.purpose = Some(purpose);
            self.signature = Some(signature);
            self.last_edit = std::time::Instant::now();
        }
    }

    pub fn leave(&mut self) {
        if self.purpose == Some(OriginalPurpose::Tool)
            || (self.purpose.is_some() && self.settled.is_none())
        {
            self.invalidate();
        } else {
            self.cancel_pending();
        }
    }

    pub fn idle(&self) -> bool {
        self.refining.is_none()
            && (self.signature.is_none()
                || self.settled.is_some()
                || self.refinement_error == Some(self.revision()))
            && self.last_edit.elapsed() >= Duration::from_millis(250)
    }

    pub fn schedule(
        &mut self,
        input: &ProcessingInput,
        target: OutputTarget,
        scope: &mut super::grade_scope::GradeScope,
        context: &egui::Context,
        source: source::Context,
    ) {
        let Some(signature) = &self.signature else {
            return;
        };
        let revision = self.revision();
        let view = signature.2;
        if self.refining.is_none()
            && self.settled.is_none()
            && self.refinement_error != Some(revision)
            && self.last_edit.elapsed() >= Duration::from_millis(250)
            && let Some(view) = view
        {
            scope.request = Some(Request {
                revision,
                generation: self.generation.clone(),
                input: input.clone(),
                target,
                view,
                source,
            });
            self.refining = Some((revision, view));
        }
        if self.refining.is_some()
            || self.settled.is_none() && self.refinement_error != Some(revision)
        {
            context.request_repaint_after(Duration::from_millis(50));
        }
    }

    pub fn receive(
        &mut self,
        revision: u64,
        result: Result<Option<Arc<GpuImage>>, String>,
    ) -> Option<String> {
        let (_, view) = self.refining.filter(|(running, _)| *running == revision)?;
        self.refining = None;
        if self.revision() != revision {
            return None;
        }
        match result {
            Ok(Some(image)) => self.settled = Some((revision, view, image)),
            Ok(None) => {}
            Err(error) => {
                self.refinement_error = Some(revision);
                return Some(format!("Original view failed: {error}"));
            }
        }
        None
    }
}
