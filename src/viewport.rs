use crate::raw::Rect;
use egui::{Pos2, Vec2};

#[derive(Clone, Copy, Debug, Default)]
pub struct Viewport {
    pub zoom: Option<f32>,
    center: Option<Vec2>,
}
#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    pub canvas: egui::Rect,
    pub image: egui::Rect,
    pub area: Rect,
    pub scale: f32,
}
impl Viewport {
    pub fn fit(&mut self) {
        *self = Self::default();
    }
    pub fn layout(&mut self, canvas: egui::Rect, area: Rect, pixels_per_point: f32) -> Geometry {
        let size = egui::vec2(area.width as f32, area.height as f32);
        let scale = self
            .zoom
            .map(|zoom| zoom / pixels_per_point)
            .unwrap_or_else(|| (canvas.width() / size.x).min(canvas.height() / size.y));
        let half = canvas.size() / scale * 0.5;
        let mut center = self.center.unwrap_or(size * 0.5);
        for axis in 0..2 {
            center[axis] = if half[axis] >= size[axis] * 0.5 {
                size[axis] * 0.5
            } else {
                center[axis].clamp(half[axis], size[axis] - half[axis])
            };
        }
        self.center = Some(center);
        Geometry {
            canvas,
            image: egui::Rect::from_center_size(
                canvas.center() + (size * 0.5 - center) * scale,
                size * scale,
            ),
            area,
            scale,
        }
    }
    pub fn zoom_at(&mut self, geometry: Geometry, pixels_per_point: f32, zoom: f32, anchor: Pos2) {
        let zoom = zoom.clamp(0.01, 16.);
        let local = (anchor - geometry.image.min) / geometry.scale;
        self.zoom = Some(zoom);
        self.center = Some(local - (anchor - geometry.canvas.center()) / (zoom / pixels_per_point));
    }
    pub fn pan(&mut self, geometry: Geometry, pixels_per_point: f32, delta: Vec2) {
        self.zoom = Some(geometry.scale * pixels_per_point);
        let size = egui::vec2(geometry.area.width as f32, geometry.area.height as f32);
        let center =
            size * 0.5 + (geometry.canvas.center() - geometry.image.center()) / geometry.scale;
        self.center = Some(center - delta / geometry.scale);
    }
}
impl Geometry {
    pub fn source_point(self, point: Pos2) -> Pos2 {
        let p = (point - self.image.min) / self.scale;
        egui::pos2(
            self.area.x as f32 + p.x.clamp(0., self.area.width as f32),
            self.area.y as f32 + p.y.clamp(0., self.area.height as f32),
        )
    }
    pub fn screen_point(self, point: Pos2) -> Pos2 {
        self.image.min + (point - egui::pos2(self.area.x as f32, self.area.y as f32)) * self.scale
    }
    pub fn screen_region(self, region: Rect) -> egui::Rect {
        egui::Rect::from_min_size(
            self.screen_point(egui::pos2(region.x as f32, region.y as f32)),
            egui::vec2(region.width as f32, region.height as f32) * self.scale,
        )
    }
    pub fn visible_area(self) -> Rect {
        let clipped = self.canvas.intersect(self.image);
        source_rectangle(
            self.area,
            self.source_point(clipped.min),
            self.source_point(clipped.max),
        )
    }
    pub fn output_size(self, region: Rect, pixels_per_point: f32) -> [u32; 2] {
        [
            ((region.width as f32 * self.scale * pixels_per_point).ceil() as u32)
                .clamp(1, region.width),
            ((region.height as f32 * self.scale * pixels_per_point).ceil() as u32)
                .clamp(1, region.height),
        ]
    }
    pub fn uv(self, draw: egui::Rect, sampled: Rect) -> [f32; 4] {
        let start = self.source_point(draw.min);
        let end = self.source_point(draw.max);
        [
            (start.x - sampled.x as f32) / sampled.width as f32,
            (start.y - sampled.y as f32) / sampled.height as f32,
            (end.x - start.x) / sampled.width as f32,
            (end.y - start.y) / sampled.height as f32,
        ]
    }
    pub fn rectangle(self, start: Pos2, end: Pos2) -> Rect {
        source_rectangle(self.area, self.source_point(start), self.source_point(end))
    }
    pub fn patch(self, point: Pos2, side: u32) -> Rect {
        let p = self.source_point(point);
        let width = side.min(self.area.width);
        let height = side.min(self.area.height);
        Rect {
            x: (p.x - width as f32 * 0.5)
                .floor()
                .max(self.area.x as f32)
                .min((self.area.x + self.area.width - width) as f32) as u32,
            y: (p.y - height as f32 * 0.5)
                .floor()
                .max(self.area.y as f32)
                .min((self.area.y + self.area.height - height) as f32) as u32,
            width,
            height,
        }
    }
}
#[derive(Clone, Copy)]
pub enum PickDrag {
    New(Pos2),
    Move { start: Pos2, original: Rect },
    Resize { fixed: Pos2 },
}
impl PickDrag {
    pub fn begin(geometry: Geometry, point: Pos2, region: Option<Rect>) -> Self {
        if let Some(region) = region {
            let screen = geometry.screen_region(region);
            for (corner, opposite) in [
                (screen.min, screen.max),
                (screen.max, screen.min),
                (screen.left_bottom(), screen.right_top()),
                (screen.right_top(), screen.left_bottom()),
            ] {
                if corner.distance(point) <= 8. {
                    return Self::Resize {
                        fixed: geometry.source_point(opposite),
                    };
                }
            }
            if screen.contains(point) {
                return Self::Move {
                    start: geometry.source_point(point),
                    original: region,
                };
            }
        }
        Self::New(geometry.source_point(point))
    }
    pub fn update(self, geometry: Geometry, point: Pos2) -> Rect {
        let point = geometry.source_point(point);
        match self {
            Self::New(start) => source_rectangle(geometry.area, start, point),
            Self::Resize { fixed } => source_rectangle(geometry.area, fixed, point),
            Self::Move { start, original } => {
                let delta = point - start;
                Rect {
                    x: (original.x as f32 + delta.x).round().clamp(
                        geometry.area.x as f32,
                        (geometry.area.x + geometry.area.width - original.width) as f32,
                    ) as u32,
                    y: (original.y as f32 + delta.y).round().clamp(
                        geometry.area.y as f32,
                        (geometry.area.y + geometry.area.height - original.height) as f32,
                    ) as u32,
                    ..original
                }
            }
        }
    }
}

fn source_rectangle(area: Rect, start: Pos2, end: Pos2) -> Rect {
    // egui's f32 round-trip can straddle an exact pixel boundary at different zooms.
    let snap = |value: f32| {
        let rounded = value.round();
        if (value - rounded).abs() <= 2. * f32::EPSILON * value.abs().max(1.) {
            rounded
        } else {
            value
        }
    };
    let start = egui::pos2(snap(start.x), snap(start.y));
    let end = egui::pos2(snap(end.x), snap(end.y));
    let x = (start.x.min(end.x).floor() as u32).clamp(area.x, area.x + area.width - 1);
    let y = (start.y.min(end.y).floor() as u32).clamp(area.y, area.y + area.height - 1);
    let right = (start.x.max(end.x).ceil() as u32).clamp(x + 1, area.x + area.width);
    let bottom = (start.y.max(end.y).ceil() as u32).clamp(y + 1, area.y + area.height);
    Rect {
        x,
        y,
        width: right - x,
        height: bottom - y,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const AREA: Rect = Rect {
        x: 12,
        y: 12,
        width: 8640,
        height: 5760,
    };
    #[test]
    fn fit_and_retina_one_to_one_have_correct_scale_and_roundtrips() {
        let canvas = egui::Rect::from_min_size(egui::pos2(50., 20.), egui::vec2(900., 600.));
        let mut view = Viewport::default();
        let fit = view.layout(canvas, AREA, 2.);
        assert_eq!(fit.image, canvas);
        assert!((fit.scale - 900. / 8640.).abs() < 1e-6);
        let p = egui::pos2(432., 2345.);
        assert!((fit.source_point(fit.screen_point(p)) - p).length() < 0.001);
        view.zoom_at(fit, 2., 1., canvas.center());
        let detail = view.layout(canvas, AREA, 2.);
        assert_eq!(detail.scale, 0.5);
        let region = detail.visible_area();
        assert_eq!((region.width, region.height), (1800, 1200));
        assert_eq!(detail.output_size(region, 2.), [1800, 1200]);
    }
    #[test]
    fn pointer_zoom_preserves_anchor_and_pan_clamps_to_source_bounds() {
        let canvas = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(900., 600.));
        let mut view = Viewport::default();
        let fit = view.layout(canvas, AREA, 2.);
        let anchor = egui::pos2(400., 240.);
        let before = fit.source_point(anchor);
        view.zoom_at(fit, 2., 1., anchor);
        let zoomed = view.layout(canvas, AREA, 2.);
        assert!((zoomed.source_point(anchor) - before).length() < 0.001);
        view.pan(zoomed, 2., egui::vec2(100000., 100000.));
        let edge = view.layout(canvas, AREA, 2.);
        let region = edge.visible_area();
        assert_eq!((region.x, region.y), (AREA.x, AREA.y));
        view.fit();
        assert_eq!(view.layout(canvas, AREA, 2.).image, canvas);
    }
    #[test]
    fn rectangle_move_and_corner_resize_keep_valid_source_bounds() {
        let canvas = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(900., 600.));
        let mut view = Viewport {
            zoom: Some(1.),
            ..Default::default()
        };
        let g = view.layout(canvas, AREA, 2.);
        let region = Rect {
            x: 4000,
            y: 2600,
            width: 128,
            height: 128,
        };
        let center = g.screen_region(region).center();
        let moved =
            PickDrag::begin(g, center, Some(region)).update(g, center + egui::vec2(10., 15.));
        assert_eq!(
            moved,
            Rect {
                x: 4020,
                y: 2630,
                ..region
            }
        );
        let corner = g.screen_region(region).max;
        let resized = PickDrag::begin(g, corner, Some(region))
            .update(g, g.screen_point(egui::pos2(4200., 2800.)));
        assert_eq!(
            resized,
            Rect {
                x: 4000,
                y: 2600,
                width: 200,
                height: 200
            }
        );
        let edge = PickDrag::begin(g, center, Some(region)).update(g, egui::pos2(100000., 100000.));
        assert_eq!(edge.x + edge.width, AREA.x + AREA.width);
        assert_eq!(edge.y + edge.height, AREA.y + AREA.height);
    }
    #[test]
    fn picker_measurements_are_source_coordinates_at_any_zoom_and_pan() {
        let canvas = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(900., 600.));
        let mut view = Viewport::default();
        let fit = view.layout(canvas, AREA, 2.);
        let a = egui::pos2(4000., 2600.);
        let b = egui::pos2(4128., 2728.);
        let expected = fit.rectangle(fit.screen_point(a), fit.screen_point(b));
        view.zoom_at(fit, 2., 1., fit.screen_point(a));
        let zoomed = view.layout(canvas, AREA, 2.);
        view.pan(zoomed, 2., egui::vec2(-12., 20.));
        let panned = view.layout(canvas, AREA, 2.);
        let actual = panned.rectangle(panned.screen_point(a), panned.screen_point(b));
        assert_eq!(
            (expected.x, expected.y, expected.width, expected.height),
            (actual.x, actual.y, actual.width, actual.height)
        );
        let patch = panned.patch(panned.image.max, 32);
        assert_eq!((patch.width, patch.height), (32, 32));
        assert_eq!(patch.x + 32, AREA.x + AREA.width);
        assert_eq!(patch.y + 32, AREA.y + AREA.height);
        let edge = panned.rectangle(panned.image.max, panned.image.max);
        assert_eq!((edge.width, edge.height), (1, 1));
    }
}
