use super::*;
use crate::viewport::CanvasLayout;

impl Editor {
    pub(super) fn navigate(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        mut geometry: CanvasLayout,
        pixels_per_point: f32,
    ) -> (CanvasLayout, bool) {
        if !ui.is_enabled() {
            return (geometry, false);
        }
        let (zoom, translation, touch, pointer, space) = ui.input(|input| {
            (
                input.zoom_delta(),
                input.translation_delta(),
                input.multi_touch(),
                input.pointer.hover_pos(),
                input.key_down(egui::Key::Space),
            )
        });
        let dust_interaction = self.tab == 1
            && (self.dust.cleanup
                || self.dust.donor_draft.is_some()
                || self.pick == Some(Pick::Dust));
        let canvas_pick = self
            .pick
            .filter(|pick| *pick != Pick::Dust || self.tab == 1);
        let gesture_inside = touch.is_some_and(|touch| geometry.canvas.contains(touch.center_pos));
        if response.hovered() || gesture_inside {
            if zoom != 1.
                && let Some(anchor) = touch
                    .map(|touch| touch.center_pos - touch.translation_delta)
                    .or(pointer)
            {
                // Touch translation moves the previous center to the current center after zoom.
                self.viewport.zoom_at(
                    geometry,
                    pixels_per_point,
                    geometry.scale * pixels_per_point * zoom,
                    anchor,
                );
                geometry = self.viewport.layout_composition(
                    geometry.canvas,
                    geometry.composition,
                    pixels_per_point,
                );
            }
            if translation != egui::Vec2::ZERO && self.viewport.zoom.is_some() && can_pan(geometry)
            {
                self.viewport.pan(geometry, pixels_per_point, translation);
                geometry = self.viewport.layout_composition(
                    geometry.canvas,
                    geometry.composition,
                    pixels_per_point,
                );
            }
        }
        let panning = response.dragged_by(egui::PointerButton::Middle)
            || response.dragged_by(egui::PointerButton::Primary)
                && (space
                    || canvas_pick.is_none()
                        && !dust_interaction
                        && self.viewport.zoom.is_some()
                        && can_pan(geometry));
        if panning && can_pan(geometry) {
            self.viewport
                .pan(geometry, pixels_per_point, response.drag_delta());
            geometry = self.viewport.layout_composition(
                geometry.canvas,
                geometry.composition,
                pixels_per_point,
            );
        }
        if canvas_pick.is_none()
            && !(self.tab == 1 && self.dust.donor_draft.is_some())
            && !space
            && response.clicked_by(egui::PointerButton::Primary)
            && let Some(point) = response
                .interact_pointer_pos()
                .filter(|point| geometry.image.contains(*point))
        {
            if let Some(id) = self.dust_repair_at(geometry, point) {
                self.select_dust_repair(id);
            } else if !dust_interaction {
                if self.viewport.zoom.is_none() {
                    self.viewport.zoom_at(geometry, pixels_per_point, 1., point);
                } else {
                    self.viewport.fit();
                }
                geometry = self.viewport.layout_composition(
                    geometry.canvas,
                    geometry.composition,
                    pixels_per_point,
                );
            }
        }
        if response.hovered() || response.dragged() {
            let focused_donor =
                self.selected_dust_repair().is_some() && self.dust_donor_regions().is_some();
            let cursor = if panning
                || focused_donor
                    && self.dust_donor_dragging()
                    && response.dragged_by(egui::PointerButton::Primary)
            {
                egui::CursorIcon::Grabbing
            } else if focused_donor
                && pointer.is_some_and(|point| self.dust_donor_at(geometry, point))
            {
                egui::CursorIcon::Grab
            } else if canvas_pick.is_none()
                && !(self.tab == 1 && self.dust.donor_draft.is_some())
                && pointer.is_some_and(|point| self.dust_repair_at(geometry, point).is_some())
            {
                egui::CursorIcon::PointingHand
            } else if self.tab == 1 && self.dust.cleanup && self.dust.donor_draft.is_none() {
                egui::CursorIcon::Crosshair
            } else if space
                || canvas_pick.is_none() && self.viewport.zoom.is_some() && can_pan(geometry)
            {
                egui::CursorIcon::Grab
            } else if let Some(pick) = canvas_pick.filter(|_| !self.busy) {
                if pick == Pick::Dust {
                    egui::CursorIcon::Crosshair
                } else {
                    pointer
                        .map(|point| {
                            picker_cursor(
                                geometry,
                                point,
                                self.region,
                                self.pick_drag,
                                response.dragged_by(egui::PointerButton::Primary),
                            )
                        })
                        .unwrap_or(egui::CursorIcon::Crosshair)
                }
            } else if pointer.is_some_and(|point| geometry.image.contains(point))
                && canvas_pick.is_none()
            {
                if self.viewport.zoom.is_none() {
                    egui::CursorIcon::ZoomIn
                } else {
                    egui::CursorIcon::ZoomOut
                }
            } else {
                egui::CursorIcon::Default
            };
            ui.ctx().set_cursor_icon(cursor);
        }
        (geometry, panning)
    }
}
fn can_pan(geometry: CanvasLayout) -> bool {
    geometry.image.width() > geometry.canvas.width()
        || geometry.image.height() > geometry.canvas.height()
}
fn picker_cursor(
    geometry: CanvasLayout,
    point: egui::Pos2,
    region: Option<Coverage>,
    active: Option<PickDrag>,
    dragging: bool,
) -> egui::CursorIcon {
    let drag = if dragging {
        active.unwrap_or_else(|| PickDrag::begin(geometry, point, region))
    } else {
        PickDrag::begin(geometry, point, region)
    };
    match drag {
        PickDrag::New(_) => egui::CursorIcon::Crosshair,
        PickDrag::Move { .. } => {
            if dragging {
                egui::CursorIcon::Grabbing
            } else {
                egui::CursorIcon::Grab
            }
        }
        PickDrag::Resize { fixed } => {
            let delta = point - geometry.screen_frame_point(fixed);
            if delta.x * delta.y >= 0. {
                egui::CursorIcon::ResizeNwSe
            } else {
                egui::CursorIcon::ResizeNeSw
            }
        }
    }
}

#[cfg(test)]
#[path = "editor_navigation_tests.rs"]
mod tests;
