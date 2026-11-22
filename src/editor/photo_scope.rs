use super::{Editor, WorkingView};

#[cfg(test)]
mod tests;

/// Visit-owned images, readiness, contextual mask editing and repair evaluation.
/// Hosted photo interactions and unfinished mask gestures end with the visit.
#[derive(Default)]
pub(super) struct PhotoScope {
    pub current_source: Option<super::SourceId>,
    pub adjustment_layers: super::AdjustmentLayerEditor,
    pub canvas: super::canvas_view::CanvasView,
    input: PhotoInput,
}

/// Captured-source consumers retire together when a visit's interpretation changes.
#[derive(Default)]
pub(super) struct PhotoInput {
    pub project_preview: super::editing::Preview<crate::project::Project>,
    pub hosted_tool: super::tool::Slot,
    pub dust: super::dust::PhotoState,
    pub masks: super::mask_editor::MaskEditor,
    pub layer_order_drag: Option<super::adjustment_layers::OrderDrag>,
    pub working_view: WorkingView,
    pub pending_preview: Option<super::thumbnails::SelectedPreview>,
    pub return_notice: Option<String>,
    pub picker: super::picking::State,
    pub analysis_view: super::analysis::ReviewView,
    pub auto_crop_view: super::auto_crop::ReviewView,
    #[cfg(test)]
    lifetime: std::sync::Arc<()>,
    #[cfg(test)]
    pub measurement_gate: Option<super::measurement::testing::Gate>,
    pub interaction_action: Option<async_executor::Task<()>>,
    pub source: super::source::State,
}

impl std::ops::Deref for PhotoScope {
    type Target = PhotoInput;
    fn deref(&self) -> &Self::Target {
        &self.input
    }
}
impl std::ops::DerefMut for PhotoScope {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.input
    }
}

impl Editor {
    #[cfg(test)]
    pub(super) fn visit_lifetime(&self) -> std::sync::Weak<()> {
        std::sync::Arc::downgrade(&self.document.visit.input.lifetime)
    }

    pub(super) fn reinterpret_photo(&mut self) {
        self.canvas_frame = Default::default();
        let working_view = self.document.visit.working_view.for_visit();
        self.document.visit.input = PhotoInput {
            working_view,
            ..Default::default()
        };
        self.document.visit.canvas.retire_input();
    }

    pub(super) fn replace_photo_scope(&mut self, source: Option<super::SourceId>) {
        self.canvas_frame = Default::default();

        let layers = if self.document.visit.current_source == source {
            self.document.visit.adjustment_layers.selection_only()
        } else {
            super::AdjustmentLayerEditor::default()
        };
        let working_view = self.document.visit.working_view.for_visit();
        // Replacing the owner drops its task handles, images and transient tools.
        self.document.visit = PhotoScope {
            input: PhotoInput {
                working_view,
                ..Default::default()
            },
            canvas: super::canvas_view::CanvasView {
                viewport: self.document.visit.canvas.viewport,
                ..Default::default()
            },
            current_source: source,
            ..Default::default()
        };
        // Temporary reference views retire gestures, not the selected layer's identity.
        self.document.visit.adjustment_layers = layers;
    }
}
