use super::*;

fn project([x, y]: [f64; 2], angle: f64) -> [f64; 2] {
    let (s, c) = angle.sin_cos();
    [c * x + s * y, -s * x + c * y]
}

pub(super) fn visible_crop(
    active: SourceRegion,
    angle: f64,
    planes: impl Iterator<Item = Plane>,
) -> Result<Crop, Error> {
    active.validate()?;
    let planes: Vec<_> = planes.collect();
    if planes
        .iter()
        .any(|p| p.iter().any(|v| !v.is_finite()) || p[0].hypot(p[1]) <= 0.)
    {
        return Err(invalid("invalid visible spatial constraints"));
    }
    solve(active, angle, &planes).ok_or(Error::UnusableSample("no visible spatial rectangle"))
}

fn arithmetic_integer(value: f64) -> f64 {
    // Unit-normal/source-basis conversions can put an exact pixel edge a few ulps away.
    // Absorb arithmetic round-off only, using the source coverage boundary allowance.
    let integer = value.round();
    if (value - integer).abs() <= 1e-9 {
        integer
    } else {
        value
    }
}

fn clip(points: Vec<[f64; 2]>, signed: impl Fn([f64; 2]) -> f64) -> Vec<[f64; 2]> {
    let mut result = vec![];
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        let da = signed(a);
        let db = signed(b);
        if da >= 0. {
            result.push(a);
        }
        if (da >= 0.) != (db >= 0.) {
            let t = da / (da - db);
            result.push([a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]);
        }
    }
    result
}

/// Maximal-area aligned rectangle about the visible polygon's bounding-box centre.
/// The centre may move when clipping exposes less picture; no corner can leave its hypothesis.
fn solve(active: SourceRegion, angle: f64, planes: &[Plane]) -> Option<Crop> {
    let x = f64::from(active.x);
    let y = f64::from(active.y);
    let right = x + f64::from(active.width);
    let bottom = y + f64::from(active.height);
    let mut polygon: Vec<_> = [[x, y], [right, y], [right, bottom], [x, bottom]]
        .into_iter()
        .map(|p| project(p, angle))
        .collect();
    for &[a, b, c] in planes {
        polygon = clip(polygon, |uv| {
            let source = project(uv, -angle);
            a * source[0] + b * source[1] - c
        });
    }
    if polygon.len() < 3 {
        return None;
    }
    let bounds: [f64; 4] = std::array::from_fn(|side| {
        let values = polygon.iter().map(|p| p[side / 2]);
        if side % 2 == 0 {
            values.fold(f64::INFINITY, f64::min)
        } else {
            values.fold(f64::NEG_INFINITY, f64::max)
        }
    });
    let mut center = [(bounds[0] + bounds[1]) / 2., (bounds[2] + bounds[3]) / 2.];
    let inside = |point: [f64; 2]| {
        (0..polygon.len()).all(|i| {
            let a = polygon[i];
            let b = polygon[(i + 1) % polygon.len()];
            (b[0] - a[0]) * (point[1] - a[1]) - (b[1] - a[1]) * (point[0] - a[0]) >= -1e-8
        })
    };
    if !inside(center) {
        center = std::array::from_fn(|axis| {
            polygon.iter().map(|p| p[axis]).sum::<f64>() / polygon.len() as f64
        });
    }
    let constraints: Vec<_> = (0..polygon.len())
        .filter_map(|i| {
            let a = polygon[i];
            let b = polygon[(i + 1) % polygon.len()];
            let n = [a[1] - b[1], b[0] - a[0]];
            let length = n[0].hypot(n[1]);
            (length > 1e-12).then(|| {
                [
                    n[0].abs() / length,
                    n[1].abs() / length,
                    (n[0] * (center[0] - a[0]) + n[1] * (center[1] - a[1])) / length,
                ]
            })
        })
        .collect();
    let fits = |w: f64, h: f64| {
        w > 0.
            && h > 0.
            && constraints
                .iter()
                .all(|&[a, b, k]| a * w + b * h <= k + 1e-8)
    };
    let mut best = [0., 0.];
    let mut consider = |w: f64, h: f64| {
        if fits(w, h) && w * h > best[0] * best[1] {
            best = [w, h];
        }
    };
    for (i, &[a, b, k]) in constraints.iter().enumerate() {
        if a > 0. && b > 0. {
            consider(k / (2. * a), k / (2. * b));
        }
        for &[c, d, l] in &constraints[i + 1..] {
            let det = a * d - b * c;
            if det.abs() > 1e-12 {
                consider((k * d - b * l) / det, (a * l - k * c) / det);
            }
        }
    }
    if best.iter().any(|&v| !v.is_finite() || 2. * v < 1.) {
        return None;
    }
    let physical_center = project(center, -angle);
    let mut crop = if angle.abs() < 1e-12 {
        let min = std::array::from_fn::<_, 2, _>(|axis| {
            arithmetic_integer(center[axis] - best[axis]).ceil()
        });
        let max = std::array::from_fn::<_, 2, _>(|axis| {
            arithmetic_integer(center[axis] + best[axis]).floor()
        });
        if (0..2).any(|axis| max[axis] - min[axis] < 1.) {
            return None;
        }
        Crop {
            center: [(min[0] + max[0]) / 2., (min[1] + max[1]) / 2.],
            size: [(max[0] - min[0]) as u32, (max[1] - min[1]) as u32],
            angle: 0.,
        }
    } else {
        Crop {
            center: physical_center,
            size: best.map(|v| arithmetic_integer(2. * v).floor() as u32),
            angle,
        }
    };
    for _ in 0..3 {
        if crop.region(active).is_ok() {
            return Some(crop);
        }
        if crop.size.iter().any(|&v| v <= 1) {
            return None;
        }
        crop.size = crop.size.map(|v| v - 1);
    }
    None
}
