use super::*;
use futures_channel::oneshot;
use std::{cell::RefCell, rc::Rc};

pub(super) mod measurement;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum View {
    Working,
    Source,
    Native,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Display {
    pub label: &'static str,
    pub view: View,
    pub area: bool,
    pub lock_layer_selection: bool,
    pub help: Option<&'static str>,
}

impl Display {
    pub const fn sample(label: &'static str, view: View) -> Self {
        Self {
            label,
            view,
            area: false,
            lock_layer_selection: false,
            help: None,
        }
    }
}

#[derive(Clone, Copy)]
struct PickView {
    source: bool,
    viewport: Viewport,
    anchored: Viewport,
}

pub(super) struct Selection {
    id: u64,
    reply: RefCell<Option<oneshot::Sender<Coverage>>>,
    cancelled: crate::cancellation::Cancellation,
}

/// Visible support and the private completion endpoint for the current interaction.
#[derive(Default)]
pub(super) struct Session {
    pub display: Option<Display>,
    view: Option<PickView>,
    pub region: Option<Coverage>,
    selection: Option<Rc<Selection>>,
    #[cfg(test)]
    pub measurement_gate: Option<measurement::testing::Gate>,
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(selection) = &self.selection {
            selection.cancelled.cancel();
            selection.reply.borrow_mut().take();
        }
    }
}

impl Session {
    pub fn activity(&self) -> Option<TaskRequest> {
        self.selection
            .as_ref()
            .filter(|selection| selection.reply.borrow().is_none())
            .map(|selection| TaskRequest::Measurement(selection.id))
    }
}

impl Editor {
    pub(super) fn active_pick(&self) -> Option<Display> {
        if self.document.flat_edit().is_some() {
            Some(flat::DISPLAY)
        } else {
            self.document.visit.picking.display
        }
    }

    pub(super) fn pick_region(&self) -> Option<Coverage> {
        self.document
            .flat_edit()
            .map_or(self.document.visit.picking.region, |reference| {
                reference.region
            })
    }

    pub(super) fn set_pick_region(&mut self, region: Option<Coverage>) {
        if let Some(reference) = self.document.flat_edit_mut() {
            reference.region = region;
        } else {
            self.document.visit.picking.region = region;
        }
    }

    pub(super) fn cancel_picker(&mut self) {
        if self.document.visit.picking.selection.is_none()
            && self.document.visit.picking.display.is_none()
        {
            return;
        }
        if let Some(selection) = self.document.visit.picking.selection.clone() {
            selection.cancelled.cancel();
            selection.reply.borrow_mut().take();
            self.finish_selection(&selection);
        } else {
            self.restore_pick_view();
        }
    }

    fn finish_selection(&mut self, selection: &Rc<Selection>) {
        if !self
            .document
            .visit
            .picking
            .selection
            .as_ref()
            .is_some_and(|current| Rc::ptr_eq(current, selection))
        {
            return;
        }
        if self.tasks.finish(TaskRequest::Measurement(selection.id)) {
            self.busy = false;
        }
        self.document.visit.picking.selection = None;
        self.restore_pick_view();
    }

    fn restore_pick_view(&mut self) {
        self.document.visit.support = Default::default();
        let view = self.document.visit.picking.view.take();
        let source = view.map_or(!self.document.input.ready(), |view| view.source);
        let source_tool = self
            .active_pick()
            .is_some_and(|display| display.view != View::Working);
        self.document.visit.picking.display = None;
        self.set_pick_region(None);
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

    pub(super) fn show_picker(&mut self, display: Display) {
        self.document.thumbnails.cancel();
        let viewport = self.document.visit.canvas.viewport;
        let source = self.document.input.ready() && self.document.source;
        self.document.visit.picking.display = Some(display);
        self.set_pick_region(None);
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

    pub(super) fn submit_picker(&mut self) -> bool {
        if let Some(selection) = self.document.visit.picking.selection.clone() {
            if let Some(region) = self.pick_region()
                && let Some(reply) = selection.reply.borrow_mut().take()
            {
                self.tasks.begin(Task::Measurement(
                    selection.id,
                    self.active_pick().expect("visible picker").label,
                ));
                self.busy = true;
                self.status = "Measuring full-resolution patch…".into();
                let _ = reply.send(region);
            }
            true
        } else {
            false
        }
    }

    #[cfg(test)]
    pub(super) fn measurement_pending(&self) -> bool {
        self.document.visit.picking.activity().is_some()
    }

    pub(super) fn retire_pick_actions(&mut self) {
        if self.document.visit.picking.selection.is_some() {
            self.cancel_picker();
        }
        if matches!(self.document.tool, Some(tool::Tool::Pick(_))) {
            self.document.tool = None;
        }
    }
}
