//! A soft posing tie-break, not measurement qualification or an exposure/noise probability.
use super::*;

#[derive(Clone, Copy, Default, Serialize)]
pub(super) struct Projection {
    pub(super) mean: f64,
    pub(super) available: usize,
}

fn scalar(capture: &Capture, p: [f64; 2]) -> Option<f64> {
    let [w, h] = capture.size;
    if p.iter().any(|v| !v.is_finite())
        || p[0] < 0.5
        || p[1] < 0.5
        || p[0] > f64::from(w) - 0.5
        || p[1] > f64::from(h) - 0.5
    {
        return None;
    }
    let [u, v] = p.map(|v| v - 0.5);
    let x = u.floor() as u32;
    let y = v.floor() as u32;
    let x1 = (x + 1).min(w - 1);
    let y1 = (y + 1).min(h - 1);
    let tx = u - f64::from(x);
    let ty = v - f64::from(y);
    let mut sum = 0.;
    for (ix, iy, weight) in [
        (x, y, (1. - tx) * (1. - ty)),
        (x1, y, tx * (1. - ty)),
        (x, y1, (1. - tx) * ty),
        (x1, y1, tx * ty),
    ] {
        if weight == 0. {
            continue;
        }
        let rgb = capture.rgb[(iy * w + ix) as usize];
        if rgb.contains(&0) || rgb.contains(&65535) {
            return None;
        }
        sum += weight * logs(rgb)?.iter().sum::<f64>() / 3.;
    }
    Some(sum)
}

pub(super) fn gradient(capture: &Capture, p: [f64; 2]) -> Option<[f64; 2]> {
    Some([
        (scalar(capture, [p[0] + 1., p[1]])? - scalar(capture, [p[0] - 1., p[1]])?) * 0.5,
        (scalar(capture, [p[0], p[1] + 1.])? - scalar(capture, [p[0], p[1] - 1.])?) * 0.5,
    ])
}

pub(super) fn measure(
    capture: &Capture,
    frame: &PictureGeometry,
    side: usize,
    evidence: &Side,
) -> Projection {
    let Some(report) = &evidence.directional else {
        return Projection::default();
    };
    let outward = perimeter::normal(frame.angle, side).map(|v| -v);
    let mut result = Projection::default();
    for station in &report.stations {
        let point = perimeter::point(frame, side, (station.index as f64 + 0.5) / 64.);
        let Some(g) = gradient(capture, point) else {
            continue;
        };
        let scale = station.noise.hypot(station.tangent);
        if !scale.is_finite() || scale <= 0. {
            continue;
        }
        result.mean += ((g[0] * outward[0] + g[1] * outward[1]) / scale).clamp(-4., 4.);
        result.available += 1;
    }
    result.mean /= result.available.max(1) as f64;
    result
}
