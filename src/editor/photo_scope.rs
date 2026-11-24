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
    pub project_preview: super::editing::Preview<crate::project::Project>,
    pub hosted_tool: super::tool::Slot,
    pub dust: super::dust::PhotoState,
    pub masks: super::mask_editor::MaskEditor,
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

impl Editor {
    #[cfg(test)]
    pub(super) fn visit_lifetime(&self) -> std::sync::Weak<()> {
        std::sync::Arc::downgrade(&self.document.visit.lifetime)
    }

    pub(super) fn reinterpret_photo(&mut self) {
        self.reset_photo_scope(self.document.visit.current_source);
    }

    pub(super) fn replace_photo_scope(&mut self, source: Option<super::SourceId>) {
        let visit = &self.document.visit;
        let viewport = visit.canvas.viewport;
        // Temporary reference views retire gestures, not the selected layer's identity.
        let layers =
            (visit.current_source == source).then(|| visit.adjustment_layers.selection_only());

        self.reset_photo_scope(source);
        self.document.visit.canvas.viewport = viewport;
        if let Some(layers) = layers {
            self.document.visit.adjustment_layers = layers;
        }
    }

    fn reset_photo_scope(&mut self, source: Option<super::SourceId>) {
        self.canvas_frame = Default::default();
        self.document.visit = PhotoScope {
            working_view: self.document.visit.working_view.for_visit(),
            current_source: source,
            ..Default::default()
        };
    }
}
