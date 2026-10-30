use super::{Editor, TaskRequest, WorkingView};

#[cfg(test)]
mod tests;

/// Visit-owned images, readiness, picker projection and repair evaluation.
/// The document's active-tool owner retires photo-bound interactions on navigation.
#[derive(Default)]
pub(super) struct PhotoScope {
    pub view: u64,
    pub dust: super::dust::Interaction,
    pub working_view: WorkingView,
    pub canvas: super::canvas_view::CanvasView,
    pub pending_preview: Option<super::thumbnails::SelectedPreview>,
    pub return_notice: Option<String>,
    pub adjustment_layer_settings: super::AdjustmentLayerSettings,
    pub photo_orientation: super::PhotoOrientation,
    pub crop: Option<super::Crop>,
    pub picking: super::picking::Session,
    pub current_source: Option<super::SourceId>,
    pub source: super::source_workflow::State,
    pub working: super::working_workflow::State,
    pub support: super::support_workflow::State,
    pub adjustment_layers: super::AdjustmentLayerEditor,
}

impl PhotoScope {
    fn new(view: u64) -> Self {
        Self {
            view,
            ..Self::default()
        }
    }

    fn activity(&self) -> Option<TaskRequest> {
        self.picking.activity().or_else(|| {
            self.source
                .pending()
                .then_some(TaskRequest::Photo(self.view))
        })
    }
}

impl Editor {
    pub(super) fn replace_photo_scope(&mut self, view: u64) {
        if self.document.is_photo_bound() {
            self.cancel_active_tool();
        }
        let activity = self.document.visit.activity();
        let layers = self.document.visit.adjustment_layers.selection_only();
        let working_view = self.document.visit.working_view.for_visit();
        // Replacing the owner drops its task handles, images and transient tools.
        self.document.visit = PhotoScope {
            working_view,
            canvas: super::canvas_view::CanvasView {
                viewport: self.document.visit.canvas.viewport,
                ..Default::default()
            },
            current_source: self.document.visit.current_source,
            crop: self.document.visit.crop,
            photo_orientation: self.document.visit.photo_orientation,
            adjustment_layer_settings: self.document.visit.adjustment_layer_settings,
            ..PhotoScope::new(view)
        };
        // Temporary reference views retire gestures, not the selected layer's identity.
        self.document.visit.adjustment_layers = layers;
        if let Some(activity) = activity
            && self.tasks.finish(activity)
        {
            self.busy = false;
        }
    }
}
