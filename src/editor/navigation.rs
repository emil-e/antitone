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
    let can_pan = viewport.can_pan(layout);
    if panning {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else if inside
        && (space || response.sense.senses_drag() && viewport.zoom.is_some() && can_pan)
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    } else if inside
        && response.sense.senses_click()
        && pointer.is_some_and(|point| layout.image.contains(point))
    {
        ui.ctx().set_cursor_icon(if viewport.zoom.is_none() {
            egui::CursorIcon::ZoomIn
        } else {
            egui::CursorIcon::ZoomOut
        });
    }
    (layout, panning)
}

impl Editor {
    pub(super) fn navigate(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        geometry: CanvasLayout,
        pixels_per_point: f32,
    ) -> (CanvasLayout, bool) {
        let relayout = |viewport: &mut Viewport| {
            viewport.layout_composition(geometry.canvas, geometry.composition, pixels_per_point)
        };
        navigate_view(
            &mut self.active_canvas_mut().viewport,
            ui,
            response,
            geometry,
            relayout,
        )
    }
}

pub(super) fn picker_cursor(
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
