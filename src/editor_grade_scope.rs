#[cfg(test)]
#[path = "editor_grade_scope_tests.rs"]
mod tests;

use super::{ProcessingInput, ProcessingSignature, original_view};

/// Active photographic content and its purpose-bound original reconstruction.
/// Published images and reusable constants are retained by their presenters/caches,
/// not by a departed continuation. Target, look and view are output demands.
#[derive(Default)]
pub(super) struct GradeScope {
    processing: Option<ProcessingSignature>,
    pub request: Option<original_view::Request>,
    pub task: Option<async_executor::Task<()>>,
}

impl GradeScope {
    pub fn matches(&self, input: &ProcessingInput) -> bool {
        let signature = Self::signature(input);
        match (&self.processing, &signature) {
            (None, None) => true,
            (Some(a), Some(b)) => a.same_photographic(b),
            _ => false,
        }
    }

    pub fn new(input: &ProcessingInput) -> Self {
        Self {
            processing: Self::signature(input),
            ..Self::default()
        }
    }

    fn signature(input: &ProcessingInput) -> Option<ProcessingSignature> {
        // Inspection has no photographic grade; its EV is an output demand.
        input.is_positive().then(|| input.signature())
    }
}

impl super::Editor {
    pub(super) fn observe_grade(&mut self, input: &ProcessingInput) {
        if !self.document.visit.grade.matches(input) {
            self.original.invalidate();
            self.document.visit.grade = GradeScope::new(input);
        }
    }
}
