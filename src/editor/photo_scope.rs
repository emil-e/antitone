use super::{Editor, Ready, TaskRequest, WorkingView, measurement};
use std::sync::Arc;

#[cfg(test)]
mod tests;

/// Transient ownership for one visit, including source readiness and tool drafts.
/// Accepted photo data remains in the document; batch tools own document sessions.
#[derive(Default)]
pub(super) struct PhotoScope {
    pub view: u64,
    pub dust: super::dust::Interaction,
    pub spatial_consumer: Option<super::spatial::Consumer>,
    pub working_view: WorkingView,
    pub viewport: super::Viewport,
    pub pending_preview: Option<super::thumbnails::SelectedPreview>,
    pub preview_rendered: Option<(
        u64,
        crate::gpu::ProcessingSignature,
        crate::frame_raster::FrameRaster,
        super::OutputTarget,
        Arc<crate::gpu::view::RenderedView>,
    )>,
    pub selection_timing: Option<Arc<super::SelectionTiming>>,
    pub return_notice: Option<String>,
    pub adjustment_layer_settings: super::AdjustmentLayerSettings,
    pub photo_orientation: super::PhotoOrientation,
    pub crop: Option<super::Crop>,
    pub last_layout: Option<(crate::viewport::CanvasLayout, f32)>,
    pub pick_drag: Option<super::PickDrag>,
    pub current_source: Option<super::SourceId>,
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
            ..Self::default()
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
        let working_view = self.document.visit.working_view.for_visit();
        // Replacing the owner drops its task handles, images and transient tools.
        self.document.visit = PhotoScope {
            working_view,
            viewport: self.document.visit.viewport,
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
