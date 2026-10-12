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
                    || self.pick.is_none() && self.viewport.zoom.is_some() && can_pan(geometry));
        if panning && can_pan(geometry) {
            self.viewport
                .pan(geometry, pixels_per_point, response.drag_delta());
            geometry = self.viewport.layout_composition(
                geometry.canvas,
                geometry.composition,
                pixels_per_point,
            );
        }
        if self.pick.is_none()
            && !space
            && response.clicked_by(egui::PointerButton::Primary)
            && let Some(point) = response
                .interact_pointer_pos()
                .filter(|point| geometry.image.contains(*point))
        {
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
        if response.hovered() || response.dragged() {
            let cursor = if panning {
                egui::CursorIcon::Grabbing
            } else if space {
                egui::CursorIcon::Grab
            } else if self.pick.is_some() && !self.busy {
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
            } else if pointer.is_some_and(|point| geometry.image.contains(point))
                && self.pick.is_none()
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
