use super::{TextAction, dialogs};
use egui::{Context, Id, Ui};

// Preserve the single-line status strip's viewer geometry while activity changes.
pub const ACTIVITY_BAR_HEIGHT: f32 = 23.;

/// Progress describes a supplied phase, not an estimate of the whole operation.
#[derive(Clone, Copy)]
pub enum Progress<'a> {
    Indeterminate,
    Measured { fraction: f32, label: &'a str },
}

/// Working-state presentation only; lifetime, placement and cancellation belong to the caller.
pub struct ProgressContent<'a> {
    pub title: &'a str,
    pub progress: Progress<'a>,
    pub cancel: Option<&'a str>,
}

fn indeterminate_segment(track: egui::Rect, time: f64) -> egui::Rect {
    // Keep a horizontal capsule inside the track, including at either turn.
    // Rectangular scissoring alone cannot mask the track's rounded end caps.
    let width = (track.width() * 0.25)
        .max(track.height())
        .min(track.width());
    let phase = (time * 0.8).rem_euclid(1.) as f32;
    let position = 1. - (2. * phase - 1.).abs();
    let left = egui::lerp(track.left()..=(track.right() - width), position);
    egui::Rect::from_min_size(
        egui::pos2(left, track.top()),
        egui::vec2(width, track.height()),
    )
}

impl ProgressContent<'_> {
    fn spinner(&self, ui: &mut Ui, size: f32) -> egui::Response {
        let response = ui.add(egui::Spinner::new().size(size));
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::ProgressIndicator,
                ui.is_enabled(),
                self.title,
            )
        });
        response
    }

    fn bar(&self, ui: &mut Ui) {
        match self.progress {
            Progress::Measured { fraction, label } => {
                let response = ui.add(egui::ProgressBar::new(fraction).text(label));
                response.widget_info(|| {
                    let mut info = egui::WidgetInfo::labeled(
                        egui::WidgetType::ProgressIndicator,
                        ui.is_enabled(),
                        self.title,
                    );
                    info.value = Some(f64::from(fraction) * 100.);
                    info.current_text_value = Some(label.into());
                    info
                });
            }
            Progress::Indeterminate => {
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 18.),
                    egui::Sense::hover(),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::ProgressIndicator,
                        ui.is_enabled(),
                        self.title,
                    )
                });
                if ui.is_rect_visible(rect) {
                    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
                    painter.rect_filled(rect, 9., ui.visuals().extreme_bg_color);
                    let segment = indeterminate_segment(rect, ui.input(|input| input.time));
                    painter.rect_filled(segment, 9., ui.visuals().selection.bg_fill);
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(50));
                }
            }
        }
    }

    fn cancel(&self, ui: &mut Ui) -> bool {
        self.cancel
            .is_some_and(|label| TextAction::new(label).show(ui).clicked())
    }

    pub fn viewer(&self, ui: &mut Ui, canvas: egui::Rect) -> egui::Response {
        let rect = egui::Rect::from_center_size(canvas.center(), egui::vec2(32., 32.));
        let response = ui.put(rect, egui::Spinner::new().size(32.));
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::ProgressIndicator,
                ui.is_enabled(),
                self.title,
            )
        });
        response
    }

    pub fn inline(&self, ui: &mut Ui) -> bool {
        match self.progress {
            Progress::Indeterminate => {
                ui.horizontal(|ui| {
                    self.spinner(ui, 16.);
                    ui.label(self.title);
                });
            }
            Progress::Measured { .. } => self.bar(ui),
        }
        self.cancel(ui)
    }

    pub fn compact(&self, ui: &mut Ui) -> bool {
        ui.horizontal(|ui| {
            let label = match self.progress {
                Progress::Indeterminate => {
                    self.spinner(ui, 16.);
                    self.title.to_owned()
                }
                Progress::Measured { fraction, label } => {
                    let response = ui.add(
                        egui::ProgressBar::new(fraction)
                            .desired_width(80.)
                            .desired_height(16.),
                    );
                    response.widget_info(|| {
                        let mut info = egui::WidgetInfo::labeled(
                            egui::WidgetType::ProgressIndicator,
                            ui.is_enabled(),
                            format!("{} {label}", self.title),
                        );
                        info.value = Some(f64::from(fraction) * 100.);
                        info
                    });
                    format!("{} · {label}", self.title)
                }
            };
            let action_width = self.cancel.map_or(0., |label| {
                TextAction::new(label).width(ui) + ui.spacing().item_spacing.x
            });
            ui.allocate_ui_with_layout(
                egui::vec2((ui.available_width() - action_width).max(1.), 18.),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.add(egui::Label::new(label).truncate());
                },
            );
            self.cancel(ui)
        })
        .inner
    }

    pub fn modal(&self, context: &Context, id: Id) -> bool {
        // No backdrop/Escape dismissal: a working view cannot silently cancel or hide a job.
        dialogs::modal(context, id, |ui| {
            ui.heading(egui::RichText::new(self.title).strong());
            self.bar(ui);
            if let Some(label) = self.cancel {
                let cancel = TextAction::new(label).show(ui);
                dialogs::focus_default(ui, &cancel);
                cancel.clicked()
            } else {
                false
            }
        })
        .inner
    }
}

#[cfg(test)]
#[path = "editor_ui_progress_tests.rs"]
mod tests;
