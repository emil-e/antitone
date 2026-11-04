use super::{Ready, Viewport, grade_scope::GradeScope};
use std::sync::Arc;

pub(super) type Preview = (
    crate::viewer::GpuProxy,
    crate::frame_raster::FrameRaster,
    Arc<crate::gpu::view::RenderedView>,
);

/// The photograph and sampling actually passed to this canvas's paint callback.
pub(super) struct Displayed {
    pub view: Arc<crate::gpu::view::RenderedView>,
    pub mapping: crate::geometry::TextureMapping,
    pub size: [u32; 2],
}

/// Image and interaction state selected by the shared canvas and viewer controls.
#[derive(Default)]
pub(super) struct CanvasView {
    pub ready: Option<Ready>,
    pub viewport: Viewport,
    pub last_layout: Option<(crate::viewport::CanvasLayout, f32)>,
    pub preview_rendered: Option<Preview>,
    pub selection_timing: Option<Arc<super::SelectionTiming>>,
    pub grade: GradeScope,
}

impl super::Editor {
    pub(super) fn active_canvas(&self) -> &CanvasView {
        &self.document.visit.canvas
    }

    pub(super) fn active_canvas_mut(&mut self) -> &mut CanvasView {
        &mut self.document.visit.canvas
    }

    pub(super) fn canvas_inspection(&self) -> bool {
        if self.fullscreen.is_some() && self.document.visit.hosted_tool.occupied() {
            self.document.source
        } else {
            self.document
                .visit
                .picker
                .display()
                .map_or(self.document.source, |display| {
                    display.view != super::picking::View::Working
                })
        }
    }
}
