use super::*;
use presentation::{FeedbackDialog, Severity};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FailureKind {
    FlatReference,
    FlatSupport,
    FlatCorrection,
    Save,
    Export,
    Look,
    LookRendering,
}

pub(super) struct Failure {
    pub kind: FailureKind,
    pub details: String,
}

impl Failure {
    pub fn presentation(&self) -> FeedbackDialog<'_> {
        let (severity, title, explanation) = match self.kind {
            FailureKind::FlatReference => (
                Severity::Error,
                "Could not prepare reference",
                "Your existing correction and photographic settings are unchanged. Check the reference file and try again.",
            ),
            FailureKind::FlatSupport => (
                Severity::Warning,
                "Area not accepted",
                "No correction has been applied. The current area and accepted settings are unchanged. Dismiss this message to adjust the rectangle.",
            ),
            FailureKind::FlatCorrection => (
                Severity::Error,
                "Could not change correction",
                "The correction change was not applied. Your existing correction, calibration and Photo adjustments are unchanged. Review the details before trying again.",
            ),
            FailureKind::Save => (
                Severity::Error,
                "Could not save project",
                "Your edits are still in memory and the destination file was not replaced. Review the details, then try saving again.",
            ),
            FailureKind::Look => (
                Severity::Error,
                "Could not prepare look",
                "Your existing look and adjustments are unchanged. Review the details, then choose another look or LUT file.",
            ),
            FailureKind::LookRendering => (
                Severity::Error,
                "Could not render look",
                "The look could not be rendered. Your selection and photographic adjustments are unchanged. Review the details before trying again.",
            ),
            FailureKind::Export => (
                Severity::Error,
                "Could not export image",
                "Nothing was exported. The project and any existing destination file are unchanged. Review the details, then try exporting again.",
            ),
        };
        FeedbackDialog {
            severity,
            title,
            explanation,
            details: &self.details,
        }
    }
}

impl Editor {
    pub(super) fn report_failure(&mut self, kind: FailureKind, details: String) {
        self.failure = Some(Failure { kind, details });
    }

    pub(super) fn resolve_failure(&mut self, completed: FailureKind) {
        if self.failure.as_ref().is_some_and(|failure| {
            failure.kind == completed
                || matches!(
                    (failure.kind, completed),
                    (FailureKind::FlatSupport, FailureKind::FlatCorrection)
                )
        }) {
            self.failure = None;
        }
    }

    pub(super) fn feedback_dialog(&mut self, context: &egui::Context, escape_available: bool) {
        if let Some(failure) = &self.failure
            && failure.presentation().show(
                context,
                egui::Id::new("operation-feedback"),
                escape_available,
            )
        {
            self.failure = None;
        }
    }
}

#[cfg(test)]
#[path = "editor_feedback_tests.rs"]
mod tests;
