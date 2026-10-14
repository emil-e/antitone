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
                ui.add(egui::ProgressBar::new(fraction).text(label));
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
                    let width = rect.width() * 0.25;
                    let phase = (ui.input(|input| input.time) * 0.8).fract() as f32;
                    let left = egui::lerp((rect.left() - width)..=rect.right(), phase);
                    let segment = egui::Rect::from_min_size(
                        egui::pos2(left, rect.top()),
                        egui::vec2(width, rect.height()),
                    );
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
            self.cancel(ui)
        })
        .inner
    }
}

#[cfg(test)]
#[path = "editor_ui_progress_tests.rs"]
mod tests;
