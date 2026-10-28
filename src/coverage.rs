//! Convex source coverage shared by reconstruction and full-resolution measurements.
use crate::{
    Error,
    geometry::{SourcePoint, SourceRegion},
};

/// Actual source-plane footprint. Bounds restrict iteration; they are not the sampled shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coverage {
    corners: [SourcePoint; 4],
}

impl Coverage {
    pub fn new(corners: [SourcePoint; 4]) -> Result<Self, Error> {
        if corners.iter().flat_map(|p| p.0).any(|v| !v.is_finite()) {
            return Err(Error::InvalidParameter("nonfinite coverage"));
        }
        let mut sign = 0.;
        for i in 0..4 {
            let a = corners[i].0;
            let b = corners[(i + 1) % 4].0;
            let c = corners[(i + 2) % 4].0;
            let cross = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
            if cross == 0. || (sign != 0. && cross.signum() != sign) {
                return Err(Error::InvalidParameter(
                    "coverage must be a nonempty convex quadrilateral",
                ));
            }
            sign = cross.signum();
        }
        Ok(Self { corners })
    }

    pub fn rectangle(region: SourceRegion) -> Self {
        let x = f64::from(region.x);
        let y = f64::from(region.y);
        let right = x + f64::from(region.width);
        let bottom = y + f64::from(region.height);
        Self {
            corners: [[x, y], [right, y], [right, bottom], [x, bottom]].map(SourcePoint),
        }
    }

    pub fn corners(self) -> [SourcePoint; 4] {
        self.corners
    }

    pub fn area(self) -> f64 {
        polygon_area(&self.corners.map(|p| p.0))
    }

    pub fn bounds(self, active: SourceRegion) -> Result<SourceRegion, Error> {
        active.validate()?;
        let min: [f64; 2] = std::array::from_fn(|axis| {
            self.corners
                .iter()
                .map(|p| p.0[axis])
                .fold(f64::INFINITY, f64::min)
        });
        let max: [f64; 2] = std::array::from_fn(|axis| {
            self.corners
                .iter()
                .map(|p| p.0[axis])
                .fold(f64::NEG_INFINITY, f64::max)
        });
        let origin = [active.x, active.y];
        let size = [active.width, active.height];
        // Only absorb arithmetic round-off at the declared source boundary, never padded coverage.
        for axis in 0..2 {
            if min[axis] < f64::from(origin[axis]) - 1e-9
                || max[axis] > f64::from(origin[axis] + size[axis]) + 1e-9
            {
                return Err(Error::InvalidParameter(
                    "coverage is outside the active source area",
                ));
            }
        }
        let lo: [u32; 2] = std::array::from_fn(|i| (min[i].floor() as u32).max(origin[i]));
        let hi: [u32; 2] = std::array::from_fn(|i| (max[i].ceil() as u32).min(origin[i] + size[i]));
        SourceRegion {
            x: lo[0],
            y: lo[1],
            width: hi[0] - lo[0],
            height: hi[1] - lo[1],
        }
        .validate()
    }

    /// Acquisition bounds including the nearest edge cells for constant source extension.
    pub fn clipped_bounds(self, active: SourceRegion) -> Result<SourceRegion, Error> {
        active.validate()?;
        let lo: [u32; 2] = std::array::from_fn(|axis| {
            let min = self
                .corners
                .iter()
                .map(|p| p.0[axis])
                .fold(f64::INFINITY, f64::min);
            let origin = if axis == 0 { active.x } else { active.y };
            let size = if axis == 0 {
                active.width
            } else {
                active.height
            };
            min.floor().clamp(origin as f64, (origin + size - 1) as f64) as u32
        });
        let hi: [u32; 2] = std::array::from_fn(|axis| {
            let max = self
                .corners
                .iter()
                .map(|p| p.0[axis])
                .fold(f64::NEG_INFINITY, f64::max);
            let origin = if axis == 0 { active.x } else { active.y };
            let size = if axis == 0 {
                active.width
            } else {
                active.height
            };
            max.ceil()
                .clamp((origin + 1) as f64, (origin + size) as f64) as u32
        });
        SourceRegion {
            x: lo[0],
            y: lo[1],
            width: hi[0] - lo[0],
            height: hi[1] - lo[1],
        }
        .validate()
    }

    pub fn exact_rectangle(self, active: SourceRegion) -> Option<SourceRegion> {
        let region = self.bounds(active).ok()?;
        if self.area() != f64::from(region.width) * f64::from(region.height) {
            return None;
        }
        let expected = Self::rectangle(region).corners;
        self.corners
            .iter()
            .all(|p| expected.contains(p))
            .then_some(region)
    }

    /// Piecewise-constant source pixels contribute their exact positive intersection area.
    pub fn weight(self, pixel: [u32; 2]) -> f64 {
        self.rectangle_weight(pixel.map(f64::from), [1.; 2])
    }

    /// Exact positive overlap with a source-cell rectangle, for nonlocal effect support.
    pub fn intersection_area(self, region: SourceRegion) -> f64 {
        self.rectangle_weight(
            [region.x as f64, region.y as f64],
            [region.width as f64, region.height as f64],
        )
    }

    pub(crate) fn rectangle_weight(self, origin: [f64; 2], size: [f64; 2]) -> f64 {
        let mut first = [[0.; 2]; 8];
        let mut second = [[0.; 2]; 8];
        for (dst, point) in first.iter_mut().zip(self.corners) {
            *dst = [point.0[0] - origin[0], point.0[1] - origin[1]];
        }
        let (mut vertices, mut next) = (&mut first, &mut second);
        let mut count = 4;
        for (axis, edge, greater) in [
            (0, 0., true),
            (0, size[0], false),
            (1, 0., true),
            (1, size[1], false),
        ] {
            if count == 0 {
                return 0.;
            }
            let inside = |p: [f64; 2]| {
                if greater {
                    p[axis] >= edge
                } else {
                    p[axis] <= edge
                }
            };
            let mut mask = 0u16;
            for (i, p) in vertices[..count].iter().enumerate() {
                mask |= u16::from(inside(*p)) << i;
            }
            // Skip only no-op planes; mixed planes retain scalar intersection arithmetic
            // and vertex order, including the exact-boundary intersection guard.
            if mask == 0 {
                return 0.;
            }
            if mask == (1u16 << count) - 1 {
                continue;
            }
            let mut a = vertices[count - 1];
            let mut ai = mask & (1 << (count - 1)) != 0;
            let mut n = 0;
            for (i, &b) in vertices[..count].iter().enumerate() {
                let bi = mask & (1 << i) != 0;
                if ai != bi && a[axis] != edge && b[axis] != edge {
                    let t = (edge - a[axis]) / (b[axis] - a[axis]);
                    next[n] = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
                    next[n][axis] = edge;
                    n += 1;
                }
                if bi {
                    next[n] = b;
                    n += 1;
                }
                a = b;
                ai = bi;
            }
            std::mem::swap(&mut vertices, &mut next);
            count = n;
        }
        polygon_area(&vertices[..count]).min(size[0] * size[1])
    }

    pub fn pixels(
        self,
        active: SourceRegion,
    ) -> Result<impl Iterator<Item = ([u32; 2], f64)>, Error> {
        let bounds = self.bounds(active)?;
        Ok((bounds.y..bounds.y + bounds.height).flat_map(move |y| {
            (bounds.x..bounds.x + bounds.width).filter_map(move |x| {
                let weight = self.weight([x, y]);
                (weight > 0.).then_some(([x, y], weight))
            })
        }))
    }

    /// Native diagnostics inspect exactly the cells contributing to the developed-pixel mean.
    pub fn row_spans(
        self,
        active: SourceRegion,
    ) -> Result<impl Iterator<Item = SourceRegion>, Error> {
        let bounds = self.bounds(active)?;
        Ok((bounds.y..bounds.y + bounds.height).filter_map(move |y| {
            let mut xs = (bounds.x..bounds.x + bounds.width).filter(|&x| self.weight([x, y]) > 0.);
            let first = xs.next()?;
            let last = xs.next_back().unwrap_or(first);
            Some(SourceRegion {
                x: first,
                y,
                width: last - first + 1,
                height: 1,
            })
        }))
    }
}

fn polygon_area(points: &[[f64; 2]]) -> f64 {
    if points.len() < 3 {
        return 0.;
    }
    let origin = points[0];
    let mut area = 0.;
    for pair in points[1..].windows(2) {
        area += (pair[0][0] - origin[0]) * (pair[1][1] - origin[1])
            - (pair[0][1] - origin[1]) * (pair[1][0] - origin[0]);
    }
    area.abs() * 0.5
}

#[cfg(test)]
mod weight_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_spans_exclude_bad_bounding_box_corners_and_include_weighted_boundaries() {
        let active = SourceRegion {
            x: 10,
            y: 20,
            width: 4,
            height: 4,
        };
        let coverage =
            Coverage::new([[12., 20.], [14., 22.], [12., 24.], [10., 22.]].map(SourcePoint))
                .unwrap();
        let bad = [[10, 20], [13, 20], [10, 23], [13, 23]];
        let diagnosed: Vec<_> = coverage
            .row_spans(active)
            .unwrap()
            .flat_map(|r| (r.x..r.x + r.width).map(move |x| [x, r.y]))
            .collect();
        assert!(bad.iter().all(|p| !diagnosed.contains(p)));
        assert!(diagnosed.contains(&[11, 20])); // Half-covered developed cell, not just its centre.
        assert_eq!(
            diagnosed,
            coverage
                .pixels(active)
                .unwrap()
                .map(|(p, _)| p)
                .collect::<Vec<_>>()
        );
        let mean =
            crate::sampling::weighted_mean_iter(coverage.pixels(active).unwrap().map(|(p, w)| {
                Ok((
                    glam::DVec3::splat(if bad.contains(&p) { 1e9 } else { 0.25 }),
                    w,
                ))
            }))
            .unwrap();
        assert_eq!(mean, glam::DVec3::splat(0.25));
    }

    #[test]
    fn malformed_or_outside_coverage_is_rejected() {
        for points in [
            [[0., 0.], [1., 1.], [1., 0.], [0., 1.]],
            [[0., 0.], [1., 0.], [1., 0.], [0., 1.]],
            [[0., 0.], [1., 0.], [1., 1.], [f64::NAN, 1.]],
        ] {
            assert!(Coverage::new(points.map(SourcePoint)).is_err());
        }
        let coverage =
            Coverage::new([[0., 0.], [2., 0.], [2., 2.], [0., 2.]].map(SourcePoint)).unwrap();
        assert!(
            coverage
                .bounds(SourceRegion {
                    x: 1,
                    y: 1,
                    width: 2,
                    height: 2
                })
                .is_err()
        );
    }

    #[test]
    fn diamond_coverage_has_independent_fractional_weights_and_matching_spans() {
        let active = SourceRegion {
            x: 10,
            y: 20,
            width: 4,
            height: 4,
        };
        let coverage =
            Coverage::new([[12., 20.], [14., 22.], [12., 24.], [10., 22.]].map(SourcePoint))
                .unwrap();
        let expected = [
            [0., 0.5, 0.5, 0.],
            [0.5, 1., 1., 0.5],
            [0.5, 1., 1., 0.5],
            [0., 0.5, 0.5, 0.],
        ];
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(
                    coverage.weight([10 + x, 20 + y]),
                    expected[y as usize][x as usize]
                );
            }
        }
        assert_eq!(coverage.area(), 8.);
        assert_eq!(
            coverage
                .pixels(active)
                .unwrap()
                .map(|(_, w)| w)
                .sum::<f64>(),
            8.
        );
        let spans: Vec<_> = coverage.row_spans(active).unwrap().collect();
        assert_eq!(
            spans,
            vec![
                SourceRegion {
                    x: 11,
                    y: 20,
                    width: 2,
                    height: 1
                },
                SourceRegion {
                    x: 10,
                    y: 21,
                    width: 4,
                    height: 1
                },
                SourceRegion {
                    x: 10,
                    y: 22,
                    width: 4,
                    height: 1
                },
                SourceRegion {
                    x: 11,
                    y: 23,
                    width: 2,
                    height: 1
                }
            ]
        );
        assert!(coverage.exact_rectangle(active).is_none());
        let rectangle = Coverage::rectangle(active);
        assert_eq!(rectangle.exact_rectangle(active), Some(active));
        assert_eq!(rectangle.pixels(active).unwrap().count(), 16);
    }
}
