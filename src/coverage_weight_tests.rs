//! Independent scalar traversal guards the optimized clip bookkeeping, not a new area formula.
use super::*;
use crate::crop::Crop;

fn scalar_weight(coverage: Coverage, pixel: [u32; 2]) -> f64 {
    let origin = pixel.map(f64::from);
    let mut vertices = [[0.; 2]; 8];
    for (dst, point) in vertices.iter_mut().zip(coverage.corners()) {
        *dst = [point.0[0] - origin[0], point.0[1] - origin[1]];
    }
    let mut count = 4;
    for (axis, edge, greater) in [(0, 0., true), (0, 1., false), (1, 0., true), (1, 1., false)] {
        if count == 0 {
            return 0.;
        }
        let mut next = [[0.; 2]; 8];
        let mut n = 0;
        let inside = |p: [f64; 2]| {
            if greater {
                p[axis] >= edge
            } else {
                p[axis] <= edge
            }
        };
        for i in 0..count {
            let a = vertices[(i + count - 1) % count];
            let b = vertices[i];
            if inside(a) != inside(b) && a[axis] != edge && b[axis] != edge {
                let t = (edge - a[axis]) / (b[axis] - a[axis]);
                next[n] = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
                next[n][axis] = edge;
                n += 1;
            }
            if inside(b) {
                next[n] = b;
                n += 1;
            }
        }
        vertices = next;
        count = n;
    }
    polygon_area(&vertices[..count]).min(1.)
}

#[test]
fn fast_plane_classification_preserves_scalar_weight_bits() {
    let active = SourceRegion::from([0, 0, 64, 64]);
    for angle in [
        0.,
        1e-12,
        -1e-12,
        0.015,
        -0.015,
        0.13,
        0.7,
        std::f64::consts::FRAC_PI_4,
    ] {
        for offset in [0., 1e-12, -1e-12, 0.125, 0.5, 0.999999999999] {
            for size in [[1, 1], [2, 3], [17, 13]] {
                let coverage = Crop {
                    center: [32. + offset, 32. - offset],
                    size,
                    angle,
                }
                .coverage()
                .unwrap();
                for y in 12..52 {
                    for x in 12..52 {
                        assert_eq!(
                            coverage.weight([x, y]).to_bits(),
                            scalar_weight(coverage, [x, y]).to_bits(),
                            "{angle} {offset} {size:?} {x} {y}"
                        );
                    }
                }
                let bounds = coverage.bounds(active).unwrap();
                let expected: Vec<_> = (bounds.y..bounds.y + bounds.height)
                    .flat_map(|y| {
                        (bounds.x..bounds.x + bounds.width).filter_map(move |x| {
                            let weight = scalar_weight(coverage, [x, y]);
                            (weight > 0.).then_some(([x, y], weight))
                        })
                    })
                    .collect();
                assert_eq!(
                    coverage.pixels(active).unwrap().collect::<Vec<_>>(),
                    expected
                );
            }
        }
    }
    for shift in [16_777_217_u32, 2_147_000_000] {
        for angle in [0., 0.015, -0.13, std::f64::consts::FRAC_PI_4] {
            let coverage = Crop {
                center: [f64::from(shift) + 4.125, f64::from(shift) + 4.5],
                size: [3, 5],
                angle,
            }
            .coverage()
            .unwrap();
            for y in shift..shift + 10 {
                for x in shift..shift + 10 {
                    assert_eq!(
                        coverage.weight([x, y]).to_bits(),
                        scalar_weight(coverage, [x, y]).to_bits()
                    );
                }
            }
        }
    }
    // Nonrectangular convex footprints retain the same ordered clipping as rotated crops.
    for corners in [
        [[1., 1.], [7., 2.], [6., 7.], [2., 6.]],
        [[0., 2.], [2., 0.], [5., 1.], [4., 5.]],
    ] {
        let coverage = Coverage::new(corners.map(SourcePoint)).unwrap();
        for y in 0..9 {
            for x in 0..9 {
                assert_eq!(
                    coverage.weight([x, y]).to_bits(),
                    scalar_weight(coverage, [x, y]).to_bits()
                );
            }
        }
    }
}
