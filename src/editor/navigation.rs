use super::*;
use crate::viewport::CanvasLayout;

// Consumption is local to the delegated response. The owner retains the original
// response, including physical release; Space/middle navigation remains available.
pub(super) fn consume_primary(response: &mut egui::Response) {
    response.sense = egui::Sense::hover();
    response
        .flags
        .remove(egui::response::Flags::CLICKED | egui::response::Flags::FAKE_PRIMARY_CLICKED);
    if !response.ctx.input(|input| input.key_down(egui::Key::Space))
        && !response.dragged_by(egui::PointerButton::Middle)
    {
        response
            .flags
            .remove(egui::response::Flags::DRAGGED | egui::response::Flags::DRAG_STARTED);
    }
}

pub(super) fn navigate_view(
    viewport: &mut Viewport,
    ui: &egui::Ui,
    response: &egui::Response,
    mut layout: CanvasLayout,
    mut relayout: impl FnMut(&mut Viewport) -> CanvasLayout,
) -> (CanvasLayout, bool) {
    if !ui.is_enabled() {
        return (layout, false);
    }
    let ppp = ui.ctx().pixels_per_point();
    let (zoom, translation, touch, pointer, space) = ui.input(|input| {
        (
            input.zoom_delta(),
            input.translation_delta(),
            input.multi_touch(),
            input.pointer.hover_pos(),
            input.key_down(egui::Key::Space),
        )
    });
    let inside =
        response.hovered() || touch.is_some_and(|touch| layout.canvas.contains(touch.center_pos));
    if inside {
        if zoom != 1.
            && let Some(anchor) = touch
                .map(|t| t.center_pos - t.translation_delta)
                .or(pointer)
        {
            viewport.zoom_at(layout, ppp, layout.scale * ppp * zoom, anchor);
            layout = relayout(viewport);
        }
        if translation != egui::Vec2::ZERO && viewport.zoom.is_some() {
            viewport.pan(layout, ppp, translation);
            layout = relayout(viewport);
        }
    }
    let can_pan = viewport.can_pan(layout);
    let panning = response.dragged_by(egui::PointerButton::Middle)
        || response.dragged_by(egui::PointerButton::Primary)
            && (space || response.sense.senses_drag() && viewport.zoom.is_some() && can_pan);
    if panning && can_pan {
        viewport.pan(layout, ppp, response.drag_delta());
        layout = relayout(viewport);
    }
    if response.sense.senses_click()
        && !space
        && response.clicked_by(egui::PointerButton::Primary)
        && let Some(point) = response
            .interact_pointer_pos()
            .filter(|p| layout.image.contains(*p))
    {
        if viewport.zoom.is_none() {
            viewport.zoom_at(layout, ppp, 1., point);
        } else {
            viewport.fit();
        }
        layout = relayout(viewport);
    }
    if panning {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else if inside
        && (space || response.sense.senses_drag() && viewport.zoom.is_some() && can_pan)
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    (layout, panning)
}

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
        let mask_pointer =
            ui.input(|input| input.pointer.press_origin().or(input.pointer.hover_pos()));
        let masking =
            self.tab == 1 && self.document.mask_edit().is_some() && self.fullscreen.is_none()
                || mask_pointer.is_some_and(|point| self.mask_pointer_interest(geometry, point));
        let captured_mask = masking && self.document.mask_edit().is_some_and(MaskEdit::dragging);
        let picking = (self.active_pick().is_some() || masking) && self.fullscreen.is_none();
        let canvas_tool =
            picking || (self.document.auto_crop_edit().is_some() && self.fullscreen.is_none());
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
        if (response.hovered() || gesture_inside) && !captured_mask {
            if zoom != 1.
                && let Some(anchor) = touch
                    .map(|touch| touch.center_pos - touch.translation_delta)
                    .or(pointer)
            {
                // Touch translation moves the previous center to the current center after zoom.
                self.active_canvas_mut().viewport.zoom_at(
                    geometry,
                    pixels_per_point,
                    geometry.scale * pixels_per_point * zoom,
                    anchor,
                );
                geometry = self.active_canvas_mut().viewport.layout_composition(
                    geometry.canvas,
                    geometry.composition,
                    pixels_per_point,
                );
            }
            if translation != egui::Vec2::ZERO
                && self.active_canvas_mut().viewport.zoom.is_some()
                && can_pan(geometry)
            {
                self.active_canvas_mut()
                    .viewport
                    .pan(geometry, pixels_per_point, translation);
                geometry = self.active_canvas_mut().viewport.layout_composition(
                    geometry.canvas,
                    geometry.composition,
                    pixels_per_point,
                );
            }
        }
        let panning = !captured_mask
            && (response.dragged_by(egui::PointerButton::Middle)
                || response.dragged_by(egui::PointerButton::Primary)
                    && (space
                        || !canvas_tool
                            && self.active_canvas_mut().viewport.zoom.is_some()
                            && can_pan(geometry)));
        if panning && can_pan(geometry) {
            self.active_canvas_mut().viewport.pan(
                geometry,
                pixels_per_point,
                response.drag_delta(),
            );
            geometry = self.active_canvas_mut().viewport.layout_composition(
                geometry.canvas,
                geometry.composition,
                pixels_per_point,
            );
        }
        if !canvas_tool
            && !space
            && response.clicked_by(egui::PointerButton::Primary)
            && let Some(point) = response
                .interact_pointer_pos()
                .filter(|point| geometry.image.contains(*point))
        {
            if self.active_canvas_mut().viewport.zoom.is_none() {
                self.active_canvas_mut()
                    .viewport
                    .zoom_at(geometry, pixels_per_point, 1., point);
            } else {
                self.active_canvas_mut().viewport.fit();
            }
            geometry = self.active_canvas_mut().viewport.layout_composition(
                geometry.canvas,
                geometry.composition,
                pixels_per_point,
            );
        }
        if response.hovered() || response.dragged() {
            let cursor = if panning {
                egui::CursorIcon::Grabbing
            } else if space
                || !canvas_tool
                    && self.active_canvas_mut().viewport.zoom.is_some()
                    && can_pan(geometry)
            {
                egui::CursorIcon::Grab
            } else if masking {
                egui::CursorIcon::Crosshair
            } else if picking && !self.busy {
                pointer
                    .map(|point| {
                        let region = if self.document.analysis_review().is_some()
                            && !response.dragged_by(egui::PointerButton::Primary)
                        {
                            self.review_area_at(geometry, point).and_then(|picture| {
                                let areas =
                                    self.review_areas(self.document.visit.current_source?)?;
                                crate::scan_regions::Areas::coverage(if picture {
                                    areas.picture?
                                } else {
                                    areas.opening?
                                })
                                .ok()
                            })
                        } else {
                            self.pick_region()
                        };
                        picker_cursor(
                            geometry,
                            point,
                            region,
                            self.active_canvas().pick_drag,
                            response.dragged_by(egui::PointerButton::Primary),
                        )
                    })
                    .unwrap_or(egui::CursorIcon::Crosshair)
            } else if pointer.is_some_and(|point| geometry.image.contains(point)) && !canvas_tool {
                if self.active_canvas_mut().viewport.zoom.is_none() {
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
        PickDrag::Resize { fixed, .. } => {
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
mod consumption_tests;
#[cfg(test)]
mod tests;
