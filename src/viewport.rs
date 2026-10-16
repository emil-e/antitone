use crate::{
    coverage::Coverage,
    frame_raster::FrameRaster,
    geometry::{
        Composition, FramePoint, PhotoOrientation, RollOrientation, SourcePoint, TextureMapping,
    },
    raw::Rect,
};
use egui::{Pos2, Vec2};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Viewport {
    pub zoom: Option<f32>,
    center: Option<FramePoint>,
    preserve_anchor: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct CanvasLayout {
    pub canvas: egui::Rect,
    pub image: egui::Rect,
    pub area: Rect,
    pub composition: Composition,
    pub scale: f32,
}

impl Viewport {
    pub fn fit(&mut self) {
        *self = Self::default();
    }

    pub fn layout(
        &mut self,
        canvas: egui::Rect,
        area: Rect,
        pixels_per_point: f32,
    ) -> CanvasLayout {
        let composition = Composition::new(
            area,
            RollOrientation::default(),
            PhotoOrientation::default(),
        )
        .expect("viewport requires a valid active area");
        self.layout_composition(canvas, composition, pixels_per_point)
    }

    pub fn layout_composition(
        &mut self,
        canvas: egui::Rect,
        composition: Composition,
        pixels_per_point: f32,
    ) -> CanvasLayout {
        let dimensions = composition.size();
        let size = egui::vec2(dimensions[0] as f32, dimensions[1] as f32);
        let scale = self
            .zoom
            .map(|zoom| zoom / pixels_per_point)
            .unwrap_or_else(|| (canvas.width() / size.x).min(canvas.height() / size.y));
        let half = canvas.size() / scale * 0.5;
        let mut center = self
            .center
            .map(|point| egui::vec2(point.0[0] as f32, point.0[1] as f32))
            .unwrap_or(size * 0.5);
        if !self.preserve_anchor {
            for axis in 0..2 {
                center[axis] = if half[axis] >= size[axis] * 0.5 {
                    size[axis] * 0.5
                } else {
                    center[axis].clamp(half[axis], size[axis] - half[axis])
                };
            }
        }
        self.center = Some(FramePoint([f64::from(center.x), f64::from(center.y)]));
        CanvasLayout {
            canvas,
            image: egui::Rect::from_center_size(
                canvas.center() + (size * 0.5 - center) * scale,
                size * scale,
            ),
            area: composition.region(),
            composition,
            scale,
        }
    }

    /// Crop editing fits the entire source once, then never clamps/refits to the shrinking frame.
    pub fn layout_crop(
        &mut self,
        canvas: egui::Rect,
        composition: Composition,
        pixels_per_point: f32,
    ) -> CanvasLayout {
        if self.zoom.is_none() {
            let corners = Coverage::rectangle(composition.area())
                .corners()
                .map(|p| composition.source_to_frame(p));
            let min: [f64; 2] = std::array::from_fn(|i| {
                corners.iter().map(|p| p.0[i]).fold(f64::INFINITY, f64::min)
            });
            let max: [f64; 2] = std::array::from_fn(|i| {
                corners
                    .iter()
                    .map(|p| p.0[i])
                    .fold(f64::NEG_INFINITY, f64::max)
            });
            let scale = (canvas.width() / (max[0] - min[0]) as f32)
                .min(canvas.height() / (max[1] - min[1]) as f32);
            self.zoom = Some(scale * pixels_per_point);
            self.center = Some(FramePoint(std::array::from_fn(|i| (min[i] + max[i]) * 0.5)));
        }
        self.preserve_anchor = true;
        self.layout_composition(canvas, composition, pixels_per_point)
    }

    /// Switches viewed domains without changing physical magnification or the source anchor.
    pub fn reframe(&mut self, previous: CanvasLayout, next: Composition, pixels_per_point: f32) {
        let source = previous
            .composition
            .frame_to_source(previous.frame_point(previous.canvas.center()));
        self.center = Some(next.source_to_frame(source));
        self.zoom = Some(previous.scale * pixels_per_point);
        self.preserve_anchor = true;
    }

    pub fn zoom_at(
        &mut self,
        geometry: CanvasLayout,
        pixels_per_point: f32,
        zoom: f32,
        anchor: Pos2,
    ) {
        self.preserve_anchor = false;
        let zoom = zoom.clamp(0.01, 16.);
        let local = (anchor - geometry.image.min) / geometry.scale;
        self.zoom = Some(zoom);
        let center = local - (anchor - geometry.canvas.center()) / (zoom / pixels_per_point);
        self.center = Some(FramePoint([f64::from(center.x), f64::from(center.y)]));
    }

    pub fn pan(&mut self, geometry: CanvasLayout, pixels_per_point: f32, delta: Vec2) {
        self.preserve_anchor = false;
        self.zoom = Some(geometry.scale * pixels_per_point);
        let dimensions = geometry.composition.size();
        let size = egui::vec2(dimensions[0] as f32, dimensions[1] as f32);
        let center =
            size * 0.5 + (geometry.canvas.center() - geometry.image.center()) / geometry.scale;
        let center = center - delta / geometry.scale;
        self.center = Some(FramePoint([f64::from(center.x), f64::from(center.y)]));
    }
}

impl CanvasLayout {
    pub fn frame_point(self, point: Pos2) -> FramePoint {
        let p = (point - self.image.min) / self.scale;
        FramePoint([f64::from(p.x), f64::from(p.y)])
    }

    pub fn screen_frame_point(self, point: FramePoint) -> Pos2 {
        self.image.min + egui::vec2(point.0[0] as f32, point.0[1] as f32) * self.scale
    }

    pub fn source_point(self, point: Pos2) -> Pos2 {
        let source = self.composition.frame_to_source(self.frame_point(point));
        egui::pos2(source.0[0] as f32, source.0[1] as f32)
    }

    pub fn screen_point(self, point: Pos2) -> Pos2 {
        let frame = self
            .composition
            .source_to_frame(SourcePoint([f64::from(point.x), f64::from(point.y)]));
        self.screen_frame_point(frame)
    }

    pub fn screen_region(self, region: Rect) -> egui::Rect {
        self.screen_coverage(Coverage::rectangle(region))
    }

    pub fn visible_area(self) -> Rect {
        let clipped = self.canvas.intersect(self.image);
        let (min, max) = self.visible_frame(clipped);
        self.composition
            .coverage(min, max)
            .unwrap()
            .bounds(self.composition.area())
            .unwrap()
    }

    fn visible_frame(self, clipped: egui::Rect) -> (FramePoint, FramePoint) {
        let size = self.composition.size();
        let min = self.frame_point(clipped.min);
        let max = self.frame_point(clipped.max);
        (
            FramePoint(std::array::from_fn(|i| {
                min.0[i].clamp(0., f64::from(size[i]))
            })),
            FramePoint(std::array::from_fn(|i| {
                max.0[i].clamp(0., f64::from(size[i]))
            })),
        )
    }

    pub fn frame_raster(self, pixels_per_point: f32) -> FrameRaster {
        let (min, max) = self.visible_frame(self.canvas.intersect(self.image));
        let output = std::array::from_fn(|i| {
            (((max.0[i] - min.0[i]) * f64::from(self.scale * pixels_per_point)).ceil() as u32)
                .clamp(1, (max.0[i] - min.0[i]).ceil() as u32)
        });
        FrameRaster::new(self.composition, min, max, output).expect("visible frame is constrained")
    }

    pub fn physical_draw(self, pixels_per_point: f32) -> egui::Rect {
        let visible = self.canvas.intersect(self.image);
        let aligned = egui::Rect::from_min_max(
            (visible.min * pixels_per_point).ceil() / pixels_per_point,
            (visible.max * pixels_per_point).floor() / pixels_per_point,
        );
        if aligned.is_positive() {
            aligned
        } else {
            visible
        }
    }

    /// Match the callback's physical pixel grid, without capping at source sampling pitch.
    pub fn physical_frame_raster(self, pixels_per_point: f32) -> FrameRaster {
        let draw = self.physical_draw(pixels_per_point);
        let (min, max) = self.visible_frame(draw);
        let output =
            [draw.width(), draw.height()].map(|v| (v * pixels_per_point).round().max(1.) as u32);
        FrameRaster::new(self.composition, min, max, output)
            .expect("visible physical frame is constrained")
    }

    pub fn frame_bounds(self, coverage: Coverage) -> (FramePoint, FramePoint) {
        let corners = coverage
            .corners()
            .map(|p| self.composition.source_to_frame(p));
        let area = self.composition.area();
        let tolerance =
            8. * f64::EPSILON * f64::from((area.x + area.width).max(area.y + area.height));
        // Resolve inverse-mapping round-off before integer selection bounds reach numeric controls.
        let snap = |v: f64| {
            if (v - v.round()).abs() <= tolerance {
                v.round()
            } else {
                v
            }
        };
        (
            FramePoint(std::array::from_fn(|i| {
                snap(corners.iter().map(|p| p.0[i]).fold(f64::INFINITY, f64::min))
            })),
            FramePoint(std::array::from_fn(|i| {
                snap(
                    corners
                        .iter()
                        .map(|p| p.0[i])
                        .fold(f64::NEG_INFINITY, f64::max),
                )
            })),
        )
    }

    pub fn screen_coverage(self, coverage: Coverage) -> egui::Rect {
        let (min, max) = self.frame_bounds(coverage);
        egui::Rect::from_min_max(self.screen_frame_point(min), self.screen_frame_point(max))
    }

    pub fn frame_selection(self, start: FramePoint, end: FramePoint) -> Coverage {
        let size = self.composition.size().map(f64::from);
        let snap = |v: f64| {
            let rounded = v.round();
            if (v - rounded).abs() <= 2. * f64::from(f32::EPSILON) * v.abs().max(1.) {
                rounded
            } else {
                v
            }
        };
        let min = FramePoint(std::array::from_fn(|i| {
            snap(start.0[i].min(end.0[i]))
                .floor()
                .clamp(0., size[i] - 1.)
        }));
        let max = FramePoint(std::array::from_fn(|i| {
            snap(start.0[i].max(end.0[i]))
                .ceil()
                .clamp(min.0[i] + 1., size[i])
        }));
        self.composition
            .coverage(min, max)
            .expect("constrained frame selection")
    }

    pub fn pick_patch(self, point: Pos2, side: u32) -> Coverage {
        let p = self.frame_point(point);
        let size = self.composition.size();
        let extent: [f64; 2] = std::array::from_fn(|i| f64::from(side.min(size[i])));
        let min = FramePoint(std::array::from_fn(|i| {
            (p.0[i] - extent[i] * 0.5)
                .floor()
                .clamp(0., f64::from(size[i]) - extent[i])
        }));
        self.composition
            .coverage(
                min,
                FramePoint(std::array::from_fn(|i| min.0[i] + extent[i])),
            )
            .expect("constrained frame patch")
    }

    pub fn output_size(self, region: Rect, pixels_per_point: f32) -> [u32; 2] {
        [
            ((region.width as f32 * self.scale * pixels_per_point).ceil() as u32)
                .clamp(1, region.width),
            ((region.height as f32 * self.scale * pixels_per_point).ceil() as u32)
                .clamp(1, region.height),
        ]
    }

    pub fn uv(self, draw: egui::Rect, sampled: Rect) -> TextureMapping {
        self.composition.texture_mapping(
            self.frame_point(draw.min),
            self.frame_point(draw.max),
            sampled,
        )
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
    New(FramePoint),
    Move {
        start: FramePoint,
        original: Coverage,
    },
    Resize {
        fixed: FramePoint,
        basis: Option<[[f64; 2]; 2]>,
    },
}

impl PickDrag {
    pub fn begin(geometry: CanvasLayout, point: Pos2, region: Option<Coverage>) -> Self {
        if let Some(region) = region {
            let corners = region
                .corners()
                .map(|p| geometry.composition.source_to_frame(p));
            let edge = [
                corners[1].0[0] - corners[0].0[0],
                corners[1].0[1] - corners[0].0[1],
            ];
            if edge[0].abs() > 1e-9 && edge[1].abs() > 1e-9 {
                for (i, corner) in corners.iter().enumerate() {
                    if geometry.screen_frame_point(*corner).distance(point) <= 8. {
                        let edge2 = [
                            corners[3].0[0] - corners[0].0[0],
                            corners[3].0[1] - corners[0].0[1],
                        ];
                        let normalize = |v: [f64; 2]| {
                            let len = v[0].hypot(v[1]);
                            v.map(|x| x / len)
                        };
                        return Self::Resize {
                            fixed: corners[(i + 2) % 4],
                            basis: Some([normalize(edge), normalize(edge2)]),
                        };
                    }
                }
            }
            let screen = geometry.screen_coverage(region);
            for (corner, opposite) in [
                (screen.min, screen.max),
                (screen.max, screen.min),
                (screen.left_bottom(), screen.right_top()),
                (screen.right_top(), screen.left_bottom()),
            ] {
                if corner.distance(point) <= 8. {
                    return Self::Resize {
                        fixed: geometry.frame_point(opposite),
                        basis: None,
                    };
                }
            }
            if screen.contains(point) {
                return Self::Move {
                    start: geometry.frame_point(point),
                    original: region,
                };
            }
        }
        Self::New(geometry.frame_point(point))
    }

    pub fn update(self, geometry: CanvasLayout, point: Pos2) -> Coverage {
        let point = geometry.frame_point(point);
        match self {
            Self::New(start) => geometry.frame_selection(start, point),
            Self::Resize { fixed, basis: None } => geometry.frame_selection(fixed, point),
            Self::Resize {
                fixed,
                basis: Some([u, v]),
            } => {
                let d = [point.0[0] - fixed.0[0], point.0[1] - fixed.0[1]];
                let determinant = u[0] * v[1] - u[1] * v[0];
                let extent = |x: f64| {
                    if x.abs() < 1. {
                        if x < 0. { -1. } else { 1. }
                    } else {
                        x
                    }
                };
                let a = extent((d[0] * v[1] - d[1] * v[0]) / determinant);
                let b = extent((u[0] * d[1] - u[1] * d[0]) / determinant);
                Coverage::new([[0., 0.], [a, 0.], [a, b], [0., b]].map(|p| {
                    geometry.composition.frame_to_source(FramePoint([
                        fixed.0[0] + u[0] * p[0] + v[0] * p[1],
                        fixed.0[1] + u[1] * p[0] + v[1] * p[1],
                    ]))
                }))
                .expect("nonempty rotated statistics rectangle")
            }
            Self::Move { start, original } => {
                let (min, max) = geometry.frame_bounds(original);
                let size = geometry.composition.size().map(f64::from);
                let delta: [f64; 2] = std::array::from_fn(|i| {
                    let wanted = (min.0[i] + point.0[i] - start.0[i]).round() - min.0[i];
                    let low = -min.0[i];
                    let high = size[i] - max.0[i];
                    if min.0[i] >= 0. && max.0[i] <= size[i] && low <= high {
                        wanted.clamp(low, high)
                    } else {
                        // Off-source statistical coverage must not jump on a click.
                        wanted
                    }
                });
                Coverage::new(original.corners().map(|p| {
                    let p = geometry.composition.source_to_frame(p);
                    geometry
                        .composition
                        .frame_to_source(FramePoint([p.0[0] + delta[0], p.0[1] + delta[1]]))
                }))
                .expect("translation preserves convex coverage")
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
    area.clamped_selection(
        SourcePoint([f64::from(snap(start.x)), f64::from(snap(start.y))]),
        SourcePoint([f64::from(snap(end.x)), f64::from(snap(end.y))]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Flips;
    const AREA: Rect = Rect {
        x: 12,
        y: 12,
        width: 8640,
        height: 5760,
    };

    #[test]
    fn rotated_sampling_resize_keeps_pose_and_fixed_corner_and_move_keeps_shape() {
        let g = Viewport::default().layout(
            egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(900., 600.)),
            AREA,
            1.,
        );
        let angle = 3f64.to_radians();
        let (s, c) = angle.sin_cos();
        let q = Coverage::new(
            [[0., 0.], [3000., 0.], [3000., 2000.], [0., 2000.]]
                .map(|[x, y]| SourcePoint([2000. + c * x - s * y, 1000. + s * x + c * y])),
        )
        .unwrap();
        let corners = q.corners();
        let start = g.screen_point(egui::pos2(corners[0].0[0] as f32, corners[0].0[1] as f32));
        let resized = PickDrag::begin(g, start, Some(q)).update(g, start + egui::vec2(10., 20.));
        let p = resized.corners();
        assert!(p.iter().any(|p| p.0 == corners[2].0));
        let u = [p[1].0[0] - p[0].0[0], p[1].0[1] - p[0].0[1]];
        let v = [p[3].0[0] - p[0].0[0], p[3].0[1] - p[0].0[1]];
        assert!((u[0] * v[0] + u[1] * v[1]).abs() < 1e-6);
        let center = g.screen_coverage(q).center();
        let moved = PickDrag::begin(g, center, Some(q)).update(g, center + egui::vec2(10., 20.));
        assert!((moved.area() - q.area()).abs() < 1e-6);
    }

    #[test]
    fn off_source_statistical_coverage_does_not_move_when_clicked() {
        let g = Viewport::default().layout(
            egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(900., 600.)),
            AREA,
            1.,
        );
        let q =
            Coverage::new([[2., 200.], [300., 200.], [300., 500.], [2., 500.]].map(SourcePoint))
                .unwrap();
        let p = g.screen_coverage(q).center();
        assert_eq!(PickDrag::begin(g, p, Some(q)).update(g, p), q);
    }

    #[test]
    fn oriented_layout_preserves_source_patches_and_zoom_anchors() {
        use crate::geometry::QuarterTurns;
        let canvas = egui::Rect::from_min_size(egui::pos2(50., 20.), egui::vec2(900., 600.));
        let patch = Rect {
            x: 4000,
            y: 2600,
            width: 128,
            height: 96,
        };
        for horizontal in [false, true] {
            for vertical in [false, true] {
                for turns in [
                    QuarterTurns::Zero,
                    QuarterTurns::One,
                    QuarterTurns::Two,
                    QuarterTurns::Three,
                ] {
                    let composition = Composition::new(
                        AREA,
                        RollOrientation {
                            horizontal,
                            vertical,
                            turns: QuarterTurns::One,
                        },
                        PhotoOrientation {
                            turns,
                            flips: Flips {
                                horizontal: true,
                                vertical: false,
                            },
                        },
                    )
                    .unwrap();
                    let mut viewport = Viewport::default();
                    let fit = viewport.layout_composition(canvas, composition, 2.);
                    assert_eq!(fit.visible_area(), AREA);
                    let rect = fit.screen_region(patch);
                    assert_eq!(fit.rectangle(rect.min, rect.max), patch);
                    let anchor = rect.center();
                    let source = fit.source_point(anchor);
                    viewport.zoom_at(fit, 2., 1., anchor);
                    let detail = viewport.layout_composition(canvas, composition, 2.);
                    assert!((detail.source_point(anchor) - source).length() < 0.002);
                    let screen = detail.screen_region(patch);
                    assert_eq!(detail.rectangle(screen.min, screen.max), patch);
                    let moved =
                        PickDrag::begin(detail, screen.center(), Some(Coverage::rectangle(patch)))
                            .update(
                                detail,
                                detail.screen_point(egui::pos2(source.x + 10., source.y + 20.)),
                            )
                            .exact_rectangle(AREA)
                            .unwrap();
                    assert_eq!(
                        moved,
                        Rect {
                            x: patch.x + 10,
                            y: patch.y + 20,
                            ..patch
                        }
                    );
                    let before = viewport;
                    // Entering source inspection or picking changes neither this exact domain nor navigation.
                    let again = viewport.layout_composition(canvas, composition, 2.);
                    assert_eq!(viewport, before);
                    assert_eq!(again.image, detail.image);
                    assert_eq!(detail.scale, 0.5);
                }
            }
        }
    }

    #[test]
    fn uncropped_reframing_keeps_magnification_and_source_anchor_even_in_letterboxed_edges() {
        use crate::{crop::Crop, geometry::QuarterTurns};
        let canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900., 600.));
        for horizontal in [false, true] {
            for vertical in [false, true] {
                for turns in [
                    QuarterTurns::Zero,
                    QuarterTurns::One,
                    QuarterTurns::Two,
                    QuarterTurns::Three,
                ] {
                    let roll = RollOrientation {
                        horizontal,
                        vertical,
                        turns: QuarterTurns::Three,
                    };
                    let photo = PhotoOrientation {
                        turns,
                        flips: Flips {
                            horizontal: true,
                            vertical: false,
                        },
                    };
                    let crop = Crop::from_region(Rect {
                        x: AREA.x,
                        y: AREA.y,
                        width: 120,
                        height: 90,
                    });
                    let framed = Composition::with_crop(AREA, roll, photo, Some(crop)).unwrap();
                    let full = Composition::new(AREA, roll, photo).unwrap();
                    for zoom in [None, Some(1.)] {
                        let mut view = Viewport::default();
                        let fit = view.layout_composition(canvas, framed, 2.);
                        if let Some(zoom) = zoom {
                            view.zoom_at(fit, 2., zoom, canvas.center());
                        }
                        let original = view.layout_composition(canvas, framed, 2.);
                        let anchor = original.source_point(canvas.center());
                        view.reframe(original, full, 2.);
                        let uncropped = view.layout_composition(canvas, full, 2.);
                        assert_eq!(uncropped.scale, original.scale);
                        assert!(
                            (uncropped.source_point(canvas.center()) - anchor).length() < 0.002
                        );
                        view.reframe(uncropped, framed, 2.);
                        let returned = view.layout_composition(canvas, framed, 2.);
                        assert_eq!(returned.scale, original.scale);
                        assert_eq!(returned.image, original.image);
                    }
                }
            }
        }
    }

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
    fn rotated_full_frame_selection_has_stable_integer_bounds_for_numeric_controls() {
        let canvas = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(900., 600.));
        for angle in [-89_f64, -45., -13.7, 0.1, 19.3, 44., 89.] {
            let crop = crate::crop::Crop {
                center: [4400.25, 2880.75],
                size: [7000, 4000],
                angle: angle.to_radians(),
            }
            .constrained(AREA)
            .unwrap();
            let composition = Composition::with_crop(
                AREA,
                RollOrientation::default(),
                PhotoOrientation::default(),
                Some(crop),
            )
            .unwrap();
            let layout = Viewport::default().layout_composition(canvas, composition, 2.);
            let max = FramePoint(composition.size().map(f64::from));
            let coverage = composition.coverage(FramePoint([0., 0.]), max).unwrap();
            assert_eq!(layout.frame_bounds(coverage), (FramePoint([0., 0.]), max));
        }
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
        let moved = PickDrag::begin(g, center, Some(Coverage::rectangle(region)))
            .update(g, center + egui::vec2(10., 15.))
            .exact_rectangle(AREA)
            .unwrap();
        assert_eq!(
            moved,
            Rect {
                x: 4020,
                y: 2630,
                ..region
            }
        );
        let corner = g.screen_region(region).max;
        let resized = PickDrag::begin(g, corner, Some(Coverage::rectangle(region)))
            .update(g, g.screen_point(egui::pos2(4200., 2800.)))
            .exact_rectangle(AREA)
            .unwrap();
        assert_eq!(
            resized,
            Rect {
                x: 4000,
                y: 2600,
                width: 200,
                height: 200
            }
        );
        let edge = PickDrag::begin(g, center, Some(Coverage::rectangle(region)))
            .update(g, egui::pos2(100000., 100000.))
            .exact_rectangle(AREA)
            .unwrap();
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
