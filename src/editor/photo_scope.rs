use super::{Editor, WorkingView};

#[cfg(test)]
mod tests;

/// Visit-owned images, readiness, contextual mask editing and repair evaluation.
/// Hosted photo interactions and unfinished mask gestures end with the visit.
#[derive(Default)]
pub(super) struct PhotoScope {
    pub view: u64,
    pub hosted_tool: super::tool::Slot,
    pub dust: super::dust::PhotoState,
    pub masks: super::mask_editor::MaskEditor,
    pub working_view: WorkingView,
    pub canvas: super::canvas_view::CanvasView,
    pub pending_preview: Option<super::thumbnails::SelectedPreview>,
    pub return_notice: Option<String>,
    pub adjustment_layer_settings: super::AdjustmentLayerSettings,
    pub photo_orientation: super::PhotoOrientation,
    pub crop: Option<super::Crop>,
    pub picker: super::picking::State,
    pub calibration_action: Option<async_executor::Task<()>>,
    pub white_balance_action: Option<async_executor::Task<()>>,
    pub current_source: Option<super::SourceId>,
    pub source: super::source::State,
    pub working: super::working_view::State,
    pub adjustment_layers: super::AdjustmentLayerEditor,
}

impl PhotoScope {
    fn new(view: u64) -> Self {
        Self {
            view,
            ..Self::default()
        }
    }
}

impl Editor {
    pub(super) fn replace_photo_scope(&mut self, view: u64) {
        self.canvas_frame = Default::default();

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
    }
}
