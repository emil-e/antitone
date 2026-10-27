use super::{Editor, Ready, TaskRequest, measurement};

#[cfg(test)]
mod tests;

/// Transient ownership for one visit, including source readiness and tool drafts.
/// Accepted photo data remains in the document; batch tools own document sessions.
#[derive(Default)]
pub(super) struct PhotoScope {
    pub view: u64,
    pub ready: Option<Ready>,
    pub source: super::source_workflow::State,
    pub measurement: Option<measurement::Pending>,
    pub grade: super::grade_scope::GradeScope,
    pub working: super::working_workflow::State,
    pub support: super::support_workflow::State,
    pub crop_edit: Option<super::CropEdit>,
    pub mask_edit: Option<super::MaskEdit>,
    pub adjustment_layers: super::AdjustmentLayerEditor,
}

impl PhotoScope {
    fn new(view: u64) -> Self {
        Self {
            view,
            ready: None,
            source: Default::default(),
            measurement: None,
            grade: Default::default(),
            working: Default::default(),
            support: Default::default(),
            crop_edit: None,
            mask_edit: None,
            adjustment_layers: Default::default(),
        }
    }

    fn activity(&self) -> Option<TaskRequest> {
        self.measurement
            .as_ref()
            .map(|pending| TaskRequest::Measurement(self.view, pending.kind))
            .or_else(|| {
                self.source
                    .pending()
                    .then_some(TaskRequest::Photo(self.view))
            })
    }
}

impl Editor {
    pub(super) fn replace_photo_scope(&mut self, view: u64) {
        let activity = self.document.visit.activity();
        let layers = self.document.visit.adjustment_layers.selection_only();
        self.invalidate_original();
        self.working.clear();
        // Replacing the owner drops its task handles, cancelling their continuations.
        self.document.visit = PhotoScope::new(view);
        // Temporary reference views retire gestures, not the selected layer's identity.
        self.document.visit.adjustment_layers = layers;
        if let Some(activity) = activity
            && self.tasks.finish(activity)
        {
            self.busy = false;
        }
    }
}
