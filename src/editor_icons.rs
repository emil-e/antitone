use super::presentation::CONTROL_HEIGHT;
use egui::{Rect, Response, Stroke, Ui, Vec2};

/// Project-authored outline artwork, drawn in logical points without a font or asset dependency.
#[derive(Clone, Copy)]
pub(super) enum Icon {
    Reset,
    Fit,
    Fullscreen,
    ExitFullscreen,
    ZoomOut,
    ZoomIn,
    Source,
    Inspection,
    Help,
    Warning,
    Pending,
    Information,
    Error,
    TurnLeft,
    TurnRight,
    FlipHorizontal,
    FlipVertical,
    Crop,
    Analyze,
    LinearGradient,
    RadialGradient,
    AddLinearGradient,
    AddRadialGradient,
    Pick,
    Dust,
    Swap,
    Guides,
    Eye,
    ChevronRight,
    ChevronDown,
}

impl Icon {
    pub(super) fn paint(self, ui: &Ui, rect: Rect, stroke: Stroke) {
        let scale = (rect.width().min(rect.height()) / 16.).min(1.);
        let origin = rect.center() - Vec2::splat(8. * scale);
        let point = |x, y| origin + egui::vec2(x, y) * scale;
        let line = |points: &[[f32; 2]]| {
            ui.painter().add(egui::Shape::line(
                points.iter().map(|[x, y]| point(*x, *y)).collect(),
                stroke,
            ));
        };
        match self {
            Self::Pending => {
                ui.painter().circle_stroke(point(8., 8.), 6., stroke);
                line(&[[8., 4.], [8., 8.], [11., 10.]]);
            }
            Self::Eye => {
                let upper = (0..=24).map(|i| {
                    let t = i as f32 / 24.;
                    point(2. + 12. * t, 8. - 4. * (std::f32::consts::PI * t).sin())
                });
                let lower = (0..=24).rev().map(|i| {
                    let t = i as f32 / 24.;
                    point(2. + 12. * t, 8. + 4. * (std::f32::consts::PI * t).sin())
                });
                ui.painter().add(egui::Shape::closed_line(
                    upper.chain(lower).collect(),
                    stroke,
                ));
                ui.painter().circle_stroke(point(8., 8.), 2.5, stroke);
            }
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
            Self::Fullscreen | Self::ExitFullscreen => {
                for (x, y) in [(1., 1.), (-1., 1.), (1., -1.), (-1., -1.)] {
                    let (tip, tail) = if matches!(self, Self::Fullscreen) {
                        (6., 2.)
                    } else {
                        (2., 6.)
                    };
                    let p = |a: f32, b: f32| [8. + x * a, 8. + y * b];
                    line(&[p(tail, tail), p(tip, tip)]);
                    let base = if matches!(self, Self::Fullscreen) {
                        tip - 3.
                    } else {
                        tip + 3.
                    };
                    line(&[p(base, tip), p(tip, tip), p(tip, base)]);
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
            Self::Warning => {
                line(&[[8., 1.], [15., 14.], [1., 14.], [8., 1.]]);
                line(&[[8., 5.], [8., 9.]]);
                ui.painter()
                    .circle_filled(point(8., 11.5), 0.8, stroke.color);
            }
            Self::Information => {
                ui.painter().circle_stroke(point(8., 8.), 7., stroke);
                line(&[[8., 7.], [8., 11.]]);
                ui.painter()
                    .circle_filled(point(8., 4.5), 0.8, stroke.color);
            }
            Self::Error => {
                ui.painter().circle_stroke(point(8., 8.), 7., stroke);
                line(&[[5., 5.], [11., 11.]]);
                line(&[[5., 11.], [11., 5.]]);
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
            Self::Analyze => {
                line(&[[2., 13.], [2., 9.]]);
                line(&[[6., 13.], [6., 5.]]);
                line(&[[10., 13.], [10., 2.]]);
                line(&[[14., 13.], [14., 7.]]);
            }
            Self::Crop => {
                line(&[[4., 1.], [4., 12.], [15., 12.]]);
                line(&[[1., 4.], [12., 4.], [12., 15.]]);
            }
            Self::AddLinearGradient | Self::AddRadialGradient => {
                let base = if matches!(self, Self::AddLinearGradient) {
                    Self::LinearGradient
                } else {
                    Self::RadialGradient
                };
                base.paint(ui, rect.translate(egui::vec2(-2., -2.)), stroke);
                line(&[[10., 13.], [16., 13.]]);
                line(&[[13., 10.], [13., 16.]]);
            }
            Self::RadialGradient => {
                ui.painter().add(egui::Shape::ellipse_stroke(
                    point(8., 8.),
                    egui::vec2(6., 4.),
                    stroke,
                ));
                ui.painter().circle_stroke(point(8., 8.), 1., stroke);
            }
            Self::LinearGradient => {
                line(&[[2., 3.], [14., 3.]]);
                line(&[[2., 13.], [14., 13.]]);
                line(&[[8., 3.], [8., 13.]]);
                for y in [3., 13.] {
                    ui.painter().circle_stroke(point(8., y), 2., stroke);
                }
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
            Self::Dust => {
                ui.painter().circle_stroke(point(5., 10.), 3., stroke);
                ui.painter().circle_stroke(point(11., 6.), 3., stroke);
                line(&[[7., 8.], [9., 7.]]);
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

fn rotation_cursor_artwork() -> [Vec<Vec2>; 3] {
    let centre = egui::vec2(0., 8.);
    let arc: Vec<_> = (0..=24)
        .map(|i| {
            let theta = std::f32::consts::PI * (13. / 12. + i as f32 * (5. / 6.) / 24.);
            centre + egui::vec2(theta.cos(), theta.sin()) * 8.
        })
        .collect();
    let head = |endpoint: Vec2, direction: f32| {
        let radial = (endpoint - centre) / 8.;
        let tangent = egui::vec2(-radial.y, radial.x) * direction;
        let normal = egui::vec2(-tangent.y, tangent.x);
        let a = endpoint + normal * 2.4;
        let b = endpoint - normal * 2.4;
        // The shaft meets the head's base; its tip continues along the endpoint tangent.
        vec![a, endpoint + tangent * 3.2, b, a]
    };
    let start = head(arc[0], -1.);
    let end = head(arc[24], 1.);
    [arc, start, end]
}

pub(super) fn rotation_cursor(
    context: &egui::Context,
    id: egui::Id,
    angle: f32,
) -> egui::CustomCursorImage {
    context.data_mut(|data| {
        if let Some((previous, image)) = data.get_temp::<(f32, egui::CustomCursorImage)>(id)
            && previous == angle
        {
            return image;
        }
        let rotation = egui::emath::Rot2::from_angle(angle);
        // Keep the arc's midpoint at the hotspot, with padding for every orientation.
        let paths = rotation_cursor_artwork().map(|path| {
            path.into_iter()
                .map(|point| Vec2::splat(15.5) + rotation * point)
                .collect::<Vec<_>>()
        });
        let mut rgba = Vec::with_capacity(31 * 31 * 4);
        for y in 0..31 {
            for x in 0..31 {
                let sample = egui::vec2(x as f32 + 0.5, y as f32 + 0.5);
                let distance = paths
                    .iter()
                    .enumerate()
                    .map(|(index, path)| {
                        let distance = path
                            .windows(2)
                            .map(|segment| {
                                let edge = segment[1] - segment[0];
                                let t = ((sample - segment[0]).dot(edge) / edge.length_sq())
                                    .clamp(0., 1.);
                                (sample - segment[0] - edge * t).length_sq()
                            })
                            .fold(f32::INFINITY, f32::min)
                            .sqrt();
                        let side = |segment: &[Vec2]| {
                            let edge = segment[1] - segment[0];
                            egui::vec2(-edge.y, edge.x).dot(sample - segment[0])
                        };
                        let inside = index > 0
                            && (path.windows(2).all(|segment| side(segment) >= 0.)
                                || path.windows(2).all(|segment| side(segment) <= 0.));
                        if inside { -distance } else { distance }
                    })
                    .fold(f32::INFINITY, f32::min);
                let outline = (2.25 - distance).clamp(0., 1.) * (200. / 255.);
                let white = (1.375 - distance).clamp(0., 1.);
                let alpha = white + outline * (1. - white);
                // CustomCursorImage uses straight RGBA, unlike egui's premultiplied colours.
                let colour = if alpha > 0. { white / alpha } else { 0. };
                let grey = (colour * 255.).round() as u8;
                rgba.extend_from_slice(&[grey, grey, grey, (alpha * 255.).round() as u8]);
            }
        }
        let image = egui::CustomCursorImage {
            rgba: rgba.into(),
            size: [31, 31],
            hotspot: [15, 15],
        };
        // Retain only the current bitmap; stable orientation also preserves the backend's Arc key.
        data.insert_temp(id, (angle, image.clone()));
        image
    })
}

pub(super) fn row<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> egui::InnerResponse<R> {
    row_at(ui, CONTROL_HEIGHT, contents)
}

pub(super) fn row_at<R>(
    ui: &mut Ui,
    height: f32,
    contents: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), height),
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
    button_at(ui, icon, enabled, name, tooltip, CONTROL_HEIGHT)
}

pub(super) fn button_at(
    ui: &mut Ui,
    icon: Icon,
    enabled: bool,
    name: &str,
    tooltip: &str,
    size: f32,
) -> Response {
    ui.push_id(name, |ui| {
        ui.add_enabled_ui(enabled, |ui| {
            let response = ui.add(
                egui::Button::new("")
                    .min_size(Vec2::splat(size))
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
    toggle_at(ui, icon, value, name, tooltip, CONTROL_HEIGHT)
}

pub(super) fn toggle_at(
    ui: &mut Ui,
    icon: Icon,
    value: &mut bool,
    name: &str,
    tooltip: &str,
    size: f32,
) -> Response {
    let response = button_at(ui, icon, true, name, tooltip, size);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_arrowheads_extend_along_endpoint_tangents_and_are_filled() {
        let [arc, start, end] = rotation_cursor_artwork();
        assert!(arc[12].length() < 1e-5);
        let context = egui::Context::default();
        let image = rotation_cursor(&context, egui::Id::new("rotation-cursor"), 0.);
        for (endpoint, neighbour, head) in [(arc[0], arc[1], start), (arc[24], arc[23], end)] {
            let base = (head[0] + head[2]) * 0.5;
            let forward = head[1] - base;
            let radial = endpoint - egui::vec2(0., 8.);
            assert!((base - endpoint).length() < 1e-5);
            assert!(forward.dot(radial).abs() < 1e-5);
            assert!(forward.dot(endpoint - neighbour) > 0.);
            assert!((forward.length() - 3.2).abs() < 1e-5);
            let inside = Vec2::splat(15.5) + (head[0] + head[1] + head[2]) / 3.;
            let index = (inside.y.floor() as usize * 31 + inside.x.floor() as usize) * 4;
            assert_eq!(&image.rgba[index..index + 4], &[255; 4]);
        }
    }
}
