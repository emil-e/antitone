use super::*;

#[derive(Clone, Copy)]
struct PickView {
    source: bool,
    viewport: Viewport,
    anchored: Viewport,
}

/// Region and view state for Analyze.
#[derive(Default)]
pub(in crate::editor) struct Session {
    pub display: Option<Display>,
    view: Option<PickView>,
    pub region: Option<Coverage>,
}

impl Editor {
    pub(in crate::editor) fn legacy_pick(&self) -> Option<Display> {
        self.document.visit.picking.display
    }

    pub(in crate::editor) fn legacy_pick_region(&self) -> Option<Coverage> {
        self.document.visit.picking.region
    }

    pub(in crate::editor) fn set_legacy_pick_region(&mut self, region: Option<Coverage>) {
        self.document.visit.picking.region = region;
    }

    pub(in crate::editor) fn cancel_legacy_picker(&mut self) {
        if self.document.visit.picking.display.is_some() {
            self.restore_pick_view();
        }
    }

    fn restore_pick_view(&mut self) {
        let view = self.document.visit.picking.view.take();
        let source = view.map_or(!self.document.input.ready(), |view| view.source);
        let source_tool = self
            .legacy_pick()
            .is_some_and(|display| display.view != View::Working);
        self.document.visit.picking.display = None;
        self.set_legacy_pick_region(None);
        self.document.visit.canvas.pick_drag = None;
        if let Some(view) = view {
            if source_tool || self.document.visit.canvas.viewport == view.anchored {
                self.document.visit.canvas.viewport = view.viewport;
            } else if let Some((layout, pixels_per_point)) = self.document.visit.canvas.last_layout
            {
                self.document.visit.canvas.viewport.reframe(
                    layout,
                    layout.composition,
                    pixels_per_point,
                );
            }
        }
        self.document.source = !self.document.input.ready() || source;
    }

    pub(in crate::editor) fn show_legacy_picker(&mut self, display: Display) {
        self.document.thumbnails.cancel();
        let viewport = self.document.visit.canvas.viewport;
        let source = self.document.input.ready() && self.document.source;
        self.document.visit.picking.display = Some(display);
        self.set_legacy_pick_region(None);
        self.document.visit.canvas.pick_drag = None;
        self.document.source = display.view != View::Working;
        if let Some((layout, pixels_per_point)) = self.document.visit.canvas.last_layout {
            let next = match display.view {
                View::Native => Composition::new(
                    layout.composition.area(),
                    RollOrientation::default(),
                    PhotoOrientation::default(),
                ),
                View::Source => Composition::new(
                    layout.composition.area(),
                    self.document.roll_orientation,
                    self.document.visit.photo_orientation,
                ),
                View::Working => Ok(layout.composition),
            }
            .expect("loaded source has a valid active area");
            if next != layout.composition {
                self.document
                    .visit
                    .canvas
                    .viewport
                    .reframe(layout, next, pixels_per_point);
            }
        }
        self.document.visit.picking.view =
            self.document
                .analysis_review()
                .is_none()
                .then_some(PickView {
                    source,
                    viewport,
                    anchored: self.document.visit.canvas.viewport,
                });
    }
}
