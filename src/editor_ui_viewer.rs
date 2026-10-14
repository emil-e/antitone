use super::*;

#[cfg(test)]
#[path = "editor_ui_viewer_tests.rs"]
mod tests;

const OVERLAY_DELAY: f64 = 2.;
const OVERLAY_FADE: f32 = 0.15;

#[derive(Clone)]
struct OverlayState {
    show_until: f64,
    rect: Option<egui::Rect>,
}

#[derive(Clone, Copy)]
pub enum ViewerBar {
    Docked,
    Overlay,
}

fn overlay_id() -> egui::Id {
    egui::Id::new("fullscreen-viewer-controls")
}

impl ViewerBar {
    pub fn reset_overlay(context: &egui::Context) {
        context.data_mut(|data| data.remove::<OverlayState>(overlay_id()));
    }

    pub fn show<R>(self, ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> Option<R> {
        match self {
            Self::Docked => Some(
                egui::Panel::bottom("viewer-bar")
                    .exact_size(VIEWER_BAR_HEIGHT)
                    .show_separator_line(false)
                    .frame(
                        egui::Frame::side_top_panel(ui.style())
                            .fill(super::super::viewer_ui::surface()),
                    )
                    .show(ui, contents)
                    .inner,
            ),
            Self::Overlay => {
                let context = ui.ctx();
                let id = overlay_id();
                let layer = egui::LayerId::new(egui::Order::Foreground, id);
                let canvas = context.content_rect();
                let time = context.input(|input| input.time);
                let mut state = context
                    .data(|data| data.get_temp::<OverlayState>(id))
                    .unwrap_or(OverlayState {
                        show_until: time + OVERLAY_DELAY,
                        rect: None,
                    });
                let focused = context
                    .memory(|memory| memory.focused())
                    .and_then(|id| context.read_response(id))
                    .is_some_and(|response| response.layer_id == layer);
                let dragging = context
                    .dragged_id()
                    .and_then(|id| context.read_response(id))
                    .is_some_and(|response| response.layer_id == layer);
                let near_controls = context.input(|input| {
                    input.pointer.hover_pos().is_some_and(|point| {
                        canvas.contains(point)
                            && (point.y >= canvas.bottom() - VIEWER_BAR_HEIGHT - 24.
                                || state.rect.is_some_and(|rect| rect.contains(point)))
                    })
                });
                let held = near_controls
                    || focused
                    || dragging
                    || context.text_edit_focused()
                    || egui::Popup::is_any_open(context)
                    || context.input(|input| input.key_pressed(egui::Key::Tab));
                if held {
                    state.show_until = time + OVERLAY_DELAY;
                }
                let visible = time < state.show_until;
                if visible && !held {
                    context.request_repaint_after(std::time::Duration::from_secs_f64(
                        state.show_until - time,
                    ));
                }
                let opacity =
                    context.animate_bool_with_time(id.with("visible"), visible, OVERLAY_FADE);
                if opacity == 0. {
                    context.data_mut(|data| data.insert_temp(id, state));
                    return None;
                }
                let enabled = ui.is_enabled() && visible;
                let width = (canvas.width() - 24.).clamp(1., 720.);
                let area = egui::Area::new(id)
                    .order(egui::Order::Foreground)
                    .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0., -12.))
                    .show(context, |ui| {
                        if !enabled {
                            ui.disable();
                        }
                        ui.set_opacity(opacity);
                        ui.set_width(width);
                        egui::Frame::popup(ui.style())
                            .show(ui, |ui| {
                                ui.set_min_height(
                                    VIEWER_BAR_HEIGHT - 2. * ui.spacing().window_margin.topf(),
                                );
                                contents(ui)
                            })
                            .inner
                    });
                state.rect = Some(area.response.rect);
                context.data_mut(|data| data.insert_temp(id, state));
                Some(area.inner)
            }
        }
    }
}

pub fn viewer_surface<R>(ui: &mut Ui, fullscreen: bool, contents: impl FnOnce(&mut Ui) -> R) -> R {
    if fullscreen {
        let canvas = ui.ctx().content_rect();
        let mut viewer = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("fullscreen-photo")
                .max_rect(canvas),
        );
        viewer.set_clip_rect(canvas);
        viewer
            .painter()
            .rect_filled(canvas, 0., super::super::viewer_ui::surface());
        contents(&mut viewer)
    } else {
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(ui.style()).fill(super::super::viewer_ui::surface()))
            .show(ui, contents)
            .inner
    }
}
