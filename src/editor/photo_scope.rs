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
    pub view: u64,
    pub project_preview: super::editing::Preview<crate::project::Project>,
    pub controls_edit: Option<super::editing::ControlEdit>,
    pub hosted_tool: super::tool::Slot,
    pub dust: super::dust::PhotoState,
    pub masks: super::mask_editor::MaskEditor,
    pub working_view: WorkingView,
    pub pending_preview: Option<super::thumbnails::SelectedPreview>,
    pub return_notice: Option<String>,
    pub picker: super::picking::State,
    #[cfg(test)]
    pub measurement_gate: Option<super::measurement::testing::Gate>,
    pub interaction_action: Option<async_executor::Task<()>>,
    pub source: super::source::State,
    pub working: super::working_view::State,
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
    pub(super) fn reinterpret_photo(&mut self, view: u64) {
        self.canvas_frame = Default::default();
        let working_view = self.document.visit.working_view.for_visit();
        self.document.visit.input = PhotoInput {
            view,
            working_view,
            ..Default::default()
        };
        self.document.visit.canvas.retire_input();
    }

    pub(super) fn replace_photo_scope(&mut self, view: u64) {
        self.canvas_frame = Default::default();

        let layers = self.document.visit.adjustment_layers.selection_only();
        let working_view = self.document.visit.working_view.for_visit();
        // Replacing the owner drops its task handles, images and transient tools.
        self.document.visit = PhotoScope {
            input: PhotoInput {
                view,
                working_view,
                ..Default::default()
            },
            canvas: super::canvas_view::CanvasView {
                viewport: self.document.visit.canvas.viewport,
                ..Default::default()
            },
            current_source: self.document.visit.current_source,
            ..Default::default()
        };
        // Temporary reference views retire gestures, not the selected layer's identity.
        self.document.visit.adjustment_layers = layers;
    }
}
