#[cfg(test)]
mod tests;

use super::{ProcessingInput, ProcessingSignature};

/// Active photographic content and its original-reconstruction state and continuation.
/// Target, look and view changes supersede output demands within this owner.
#[derive(Default)]
pub(super) struct GradeScope {
    processing: Option<ProcessingSignature>,
    pub original: super::OriginalView,
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
        if !self.active_canvas().grade.matches(input) {
            self.active_canvas_mut().grade = GradeScope::new(input);
        }
    }
}
