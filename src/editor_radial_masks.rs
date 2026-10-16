use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Handle {
    New,
    Center,
    Axis(usize, f64),
    Rotate,
    Feather,
}

pub(super) struct RadialDrag {
    handle: Handle,
    point: SourcePoint,
    mask: Option<GradientMask>,
    layout: CanvasLayout,
    feather: f64,
}

fn axes(angle: f64) -> [DVec2; 2] {
    let (sin, cos) = angle.sin_cos();
    [DVec2::new(cos, sin), DVec2::new(-sin, cos)]
}
fn position(center: SourcePoint, radii: [f64; 2], angle: f64, unit: DVec2) -> SourcePoint {
    let [x, y] = axes(angle);
    SourcePoint(
        (DVec2::from_array(center.0) + x * radii[0] * unit.x + y * radii[1] * unit.y).to_array(),
    )
}
fn normalized_radius(point: SourcePoint, center: SourcePoint, radii: [f64; 2], angle: f64) -> f64 {
    let delta = DVec2::from_array(point.0) - DVec2::from_array(center.0);
    let [x, y] = axes(angle);
    (delta.dot(x) / radii[0]).hypot(delta.dot(y) / radii[1])
}
fn handles(mask: GradientMask, layout: CanvasLayout) -> Vec<(egui::Pos2, Handle)> {
    let GradientMask::Radial {
        center,
        radii,
        angle,
        feather,
    } = mask
    else {
        return Vec::new();
    };
    let screen = screen_point(layout, center);
    let x_end = screen_point(layout, position(center, radii, angle, DVec2::X));
    let x_direction = x_end - screen;
    let mut result = vec![(screen, Handle::Center)];
    for axis in 0..2 {
        for sign in [-1., 1.] {
            let unit = if axis == 0 {
                DVec2::new(sign, 0.)
            } else {
                DVec2::new(0., sign)
            };
            result.push((
                screen_point(layout, position(center, radii, angle, unit)),
                Handle::Axis(axis, sign),
            ));
        }
    }
    if x_direction.is_finite()
        && x_direction.length_sq().is_finite()
        && x_direction.length_sq() > 0.
    {
        result.push((x_end + x_direction.normalized() * 28., Handle::Rotate));
    }
    // At full feather the inner boundary collapses; the slider remains available.
    if feather < 1. {
        let diagonal = DVec2::splat(std::f64::consts::FRAC_1_SQRT_2 * (1. - feather));
        result.push((
            screen_point(layout, position(center, radii, angle, diagonal)),
            Handle::Feather,
        ));
    }
    result
}
pub(super) fn hit(mask: GradientMask, layout: CanvasLayout, point: egui::Pos2) -> bool {
    handle_at(Some(mask), layout, point) != Handle::New
}
fn handle_at(mask: Option<GradientMask>, layout: CanvasLayout, point: egui::Pos2) -> Handle {
    let Some(
        mask @ GradientMask::Radial {
            center,
            radii,
            angle,
            ..
        },
    ) = mask
    else {
        return Handle::New;
    };
    if radii
        .iter()
        .any(|radius| !radius.is_finite() || *radius <= 0.)
    {
        return Handle::New;
    }
    if let Some((_, handle)) = handles(mask, layout)
        .into_iter()
        .filter(|(position, _)| position.is_finite() && position.distance(point) <= 9.)
        .min_by(|(a, _), (b, _)| a.distance_sq(point).total_cmp(&b.distance_sq(point)))
    {
        return handle;
    }
    if normalized_radius(source_point(layout, point), center, radii, angle) < 1. {
        Handle::Center
    } else {
        Handle::New
    }
}
impl RadialDrag {
    pub(super) fn begin(
        mask: Option<GradientMask>,
        layout: CanvasLayout,
        point: egui::Pos2,
        feather: f64,
    ) -> Self {
        Self {
            handle: handle_at(mask, layout, point),
            point: source_point(layout, point),
            mask,
            layout,
            feather,
        }
    }
    pub(super) fn update(&self, point: egui::Pos2) -> GradientMask {
        let next = source_point(self.layout, point);
        if next == self.point
            && let Some(mask) = self.mask
        {
            return mask;
        }
        if let Some(GradientMask::Radial {
            mut center,
            mut radii,
            mut angle,
            mut feather,
        }) = self.mask
            && self.handle != Handle::New
        {
            let delta = DVec2::from_array(next.0) - DVec2::from_array(self.point.0);
            match self.handle {
                Handle::Center => {
                    center = SourcePoint((DVec2::from_array(center.0) + delta).to_array())
                }
                Handle::Axis(axis, sign) => {
                    radii[axis] = (radii[axis] + sign * delta.dot(axes(angle)[axis])).abs()
                }
                Handle::Rotate => {
                    let before = DVec2::from_array(self.point.0) - DVec2::from_array(center.0);
                    let after = DVec2::from_array(next.0) - DVec2::from_array(center.0);
                    if before.length_squared() > 0. && after.length_squared() > 0. {
                        let rotation = after.y.atan2(after.x) - before.y.atan2(before.x);
                        angle += rotation.sin().atan2(rotation.cos());
                    }
                }
                Handle::Feather => {
                    let before = normalized_radius(self.point, center, radii, angle);
                    let after = normalized_radius(next, center, radii, angle);
                    feather = (feather + before - after).clamp(0., 1.);
                }
                Handle::New => unreachable!(),
            }
            return GradientMask::Radial {
                center,
                radii,
                angle,
                feather,
            };
        }
        let start = self.layout.composition.source_to_frame(self.point);
        let end = self.layout.composition.source_to_frame(next);
        let horizontal = self
            .layout
            .composition
            .frame_to_source(crate::geometry::FramePoint([start.0[0] + 1., start.0[1]]));
        let direction = DVec2::from_array(horizontal.0) - DVec2::from_array(self.point.0);
        GradientMask::Radial {
            center: self.point,
            radii: [(end.0[0] - start.0[0]).hypot(end.0[1] - start.0[1]); 2],
            angle: direction.y.atan2(direction.x),
            feather: self.feather,
        }
    }
}

pub(super) fn paint(painter: &egui::Painter, layout: CanvasLayout, mask: GradientMask) {
    let GradientMask::Radial {
        center,
        radii,
        angle,
        feather,
    } = mask
    else {
        return;
    };
    let screen = screen_point(layout, center);
    let x_end = screen_point(layout, position(center, radii, angle, DVec2::X));
    let y_end = screen_point(layout, position(center, radii, angle, DVec2::Y));
    if ![screen, x_end, y_end].iter().all(|point| point.is_finite()) {
        return;
    }
    let radius = screen.distance(x_end).max(screen.distance(y_end));
    if !radius.is_finite() {
        return;
    }
    // This bounds UI-guide tessellation only; photographic masks remain analytic.
    let segments = (std::f32::consts::PI * (radius / 0.5).sqrt())
        .ceil()
        .clamp(32., 1024.) as usize;
    let stroke = egui::Stroke::new(1., egui::Color32::WHITE);
    for scale in [1., 1. - feather] {
        if scale == 0. {
            continue;
        }
        let points: Vec<_> = (0..=segments)
            .map(|index| {
                let theta = std::f64::consts::TAU * index as f64 / segments as f64;
                screen_point(
                    layout,
                    position(
                        center,
                        radii,
                        angle,
                        DVec2::new(theta.cos(), theta.sin()) * scale,
                    ),
                )
            })
            .collect();
        if points.iter().all(|point| point.is_finite()) {
            painter.add(egui::Shape::line(points, stroke));
        }
        if feather == 0. {
            break;
        }
    }
    for (point, handle) in handles(mask, layout) {
        if !point.is_finite() {
            continue;
        }
        match handle {
            Handle::Axis(..) => {
                painter.rect(
                    egui::Rect::from_center_size(point, egui::vec2(7., 7.)),
                    0.,
                    egui::Color32::BLACK,
                    stroke,
                    egui::StrokeKind::Middle,
                );
            }
            Handle::Feather => {
                let points = [
                    egui::vec2(0., -5.),
                    egui::vec2(5., 0.),
                    egui::vec2(0., 5.),
                    egui::vec2(-5., 0.),
                ]
                .map(|delta| point + delta)
                .to_vec();
                painter.add(egui::Shape::convex_polygon(
                    points,
                    egui::Color32::BLACK,
                    stroke,
                ));
            }
            Handle::Center | Handle::Rotate => {
                if handle == Handle::Rotate {
                    painter.line_segment([x_end, point], stroke);
                }
                painter.circle_filled(point, 4., egui::Color32::BLACK);
                painter.circle_stroke(point, 4., stroke);
            }
            Handle::New => {}
        }
    }
}

#[cfg(test)]
#[path = "editor_radial_masks_tests.rs"]
mod tests;
