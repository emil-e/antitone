use egui::{Rect, Response, Stroke, Ui, Vec2};

/// Project-authored outline artwork, drawn in logical points without a font or asset dependency.
#[derive(Clone, Copy)]
pub(super) enum Icon {
    Reset,
    Fit,
    ZoomOut,
    ZoomIn,
    Source,
    Inspection,
    Help,
    TurnLeft,
    TurnRight,
    FlipHorizontal,
    FlipVertical,
    Crop,
    Pick,
    Swap,
    Guides,
    ChevronRight,
    ChevronDown,
}

impl Icon {
    pub(super) fn paint(self, ui: &Ui, rect: Rect, stroke: Stroke) {
        let origin = rect.center() - Vec2::splat(8.);
        let point = |x, y| origin + egui::vec2(x, y);
        let line = |points: &[[f32; 2]]| {
            ui.painter().add(egui::Shape::line(
                points.iter().map(|[x, y]| point(*x, *y)).collect(),
                stroke,
            ));
        };
        match self {
            Self::Reset => {
                let arc = (0..=24)
                    .map(|i| {
                        let angle = -2.5 + i as f32 * 5.3 / 24.;
                        point(8. + 5.5 * angle.cos(), 8. + 5.5 * angle.sin())
                    })
                    .collect();
                ui.painter().add(egui::Shape::line(arc, stroke));
                line(&[[3.5, 1.5], [3.5, 5.5], [7.5, 5.5]]);
            }
            Self::Fit => {
                for points in [
                    [[6., 2.], [2., 2.], [2., 6.]],
                    [[10., 2.], [14., 2.], [14., 6.]],
                    [[2., 10.], [2., 14.], [6., 14.]],
                    [[14., 10.], [14., 14.], [10., 14.]],
                ] {
                    line(&points);
                }
            }
            Self::ZoomOut | Self::ZoomIn => {
                line(&[[3., 8.], [13., 8.]]);
                if matches!(self, Self::ZoomIn) {
                    line(&[[8., 3.], [8., 13.]]);
                }
            }
            Self::Source => {
                line(&[
                    [1., 8.],
                    [4., 4.],
                    [8., 2.],
                    [12., 4.],
                    [15., 8.],
                    [12., 12.],
                    [8., 14.],
                    [4., 12.],
                    [1., 8.],
                ]);
                ui.painter().circle_stroke(point(8., 8.), 2.5, stroke);
            }
            Self::Inspection => {
                for (x, y) in [(3., 5.), (8., 11.), (13., 6.)] {
                    line(&[
                        [x, 1.],
                        [x, y - 2.],
                        [x - 2., y - 2.],
                        [x - 2., y + 2.],
                        [x + 2., y + 2.],
                        [x + 2., y - 2.],
                        [x, y - 2.],
                    ]);
                    line(&[[x, y + 2.], [x, 15.]]);
                }
            }
            Self::Help => {
                ui.painter().circle_stroke(point(8., 8.), 7., stroke);
                line(&[
                    [5., 5.],
                    [6., 3.5],
                    [10., 3.5],
                    [11., 5.],
                    [11., 6.],
                    [8., 8.],
                    [8., 10.],
                ]);
                ui.painter()
                    .circle_filled(point(8., 12.5), 0.8, stroke.color);
            }
            Self::TurnLeft | Self::TurnRight => {
                let mirror = matches!(self, Self::TurnRight);
                let mapped = |points: &[[f32; 2]]| {
                    line(
                        &points
                            .iter()
                            .map(|[x, y]| [if mirror { 16. - x } else { *x }, *y])
                            .collect::<Vec<_>>(),
                    );
                };
                mapped(&[[2., 5.], [10., 5.], [13., 8.], [13., 13.]]);
                mapped(&[[5., 2.], [2., 5.], [5., 8.]]);
                mapped(&[[3., 10.], [8., 10.], [8., 15.], [3., 15.], [3., 10.]]);
            }
            Self::FlipHorizontal | Self::FlipVertical => {
                let vertical = matches!(self, Self::FlipVertical);
                let mapped = |points: &[[f32; 2]]| {
                    line(
                        &points
                            .iter()
                            .map(|[x, y]| if vertical { [*y, *x] } else { [*x, *y] })
                            .collect::<Vec<_>>(),
                    );
                };
                mapped(&[[8., 1.], [8., 15.]]);
                mapped(&[[5., 3.], [1., 13.], [5., 13.], [5., 3.]]);
                mapped(&[[11., 3.], [15., 13.], [11., 13.], [11., 3.]]);
            }
            Self::Crop => {
                line(&[[4., 1.], [4., 12.], [15., 12.]]);
                line(&[[1., 4.], [12., 4.], [12., 15.]]);
            }
            Self::Pick => {
                line(&[
                    [3., 10.],
                    [9., 4.],
                    [12., 7.],
                    [6., 13.],
                    [2., 14.],
                    [3., 10.],
                ]);
                line(&[[8., 3.], [13., 8.]]);
                line(&[[10., 4.], [12., 2.], [14., 4.], [12., 6.]]);
            }
            Self::Swap => {
                line(&[[2., 5.], [14., 5.], [11., 2.]]);
                line(&[[14., 11.], [2., 11.], [5., 14.]]);
            }
            Self::Guides => {
                line(&[[1., 1.], [15., 1.], [15., 15.], [1., 15.], [1., 1.]]);
                for n in [5.5, 10.5] {
                    line(&[[n, 1.], [n, 15.]]);
                    line(&[[1., n], [15., n]]);
                }
            }
            Self::ChevronRight => line(&[[6., 3.], [11., 8.], [6., 13.]]),
            Self::ChevronDown => line(&[[3., 6.], [8., 11.], [13., 6.]]),
        }
    }
}

pub(super) fn row<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> egui::InnerResponse<R> {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), 28.),
        egui::Layout::left_to_right(egui::Align::Center),
        contents,
    )
}

pub(super) fn button(
    ui: &mut Ui,
    icon: Icon,
    enabled: bool,
    name: &str,
    tooltip: &str,
) -> Response {
    ui.push_id(name, |ui| {
        ui.add_enabled_ui(enabled, |ui| {
            let response = ui.add(
                egui::Button::new("")
                    .min_size(Vec2::splat(28.))
                    .frame_when_inactive(false),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, response.enabled(), name)
            });
            icon.paint(
                ui,
                response.rect,
                Stroke::new(1.5, ui.style().interact(&response).fg_stroke.color),
            );
            response.on_hover_text(tooltip)
        })
        .inner
    })
    .inner
}

pub(super) fn toggle(
    ui: &mut Ui,
    icon: Icon,
    value: &mut bool,
    name: &str,
    tooltip: &str,
) -> Response {
    let response = button(ui, icon, true, name, tooltip);
    if response.clicked() {
        *value = !*value;
    }
    if *value {
        ui.painter().rect_stroke(
            response.rect.shrink(1.),
            4.,
            Stroke::new(1., ui.visuals().selection.stroke.color),
            egui::StrokeKind::Inside,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, response.enabled(), *value, name)
    });
    response
}
