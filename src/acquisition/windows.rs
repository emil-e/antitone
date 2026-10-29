//! Conservative affine support and bounded submission windows; neither changes sampling quality.

pub(crate) type Window = ([u32; 2], [u32; 2]);

#[cfg(any(test, all(feature = "gpu", feature = "render")))]
pub(crate) fn affine_output(
    affine: [[f32; 2]; 3],
    output: [u32; 2],
    origin: [u32; 2],
    tile: [u32; 2],
) -> Window {
    affine_output_rect(
        affine,
        output,
        origin.map(f64::from),
        std::array::from_fn(|i| f64::from(origin[i]) + f64::from(tile[i])),
    )
}

pub(crate) fn affine_output_rect(
    affine: [[f32; 2]; 3],
    output: [u32; 2],
    low: [f64; 2],
    high: [f64; 2],
) -> Window {
    let [start, dx, dy] = affine.map(|v| v.map(f64::from));
    let det = dx[0] * dy[1] - dx[1] * dy[0];
    debug_assert!(det.is_finite() && det != 0.);
    let points = [
        [low[0], low[1]],
        [high[0], low[1]],
        [low[0], high[1]],
        [high[0], high[1]],
    ]
    .map(|p| {
        let p = [p[0] - start[0], p[1] - start[1]];
        [
            (p[0] * dy[1] - p[1] * dy[0]) / det,
            (dx[0] * p[1] - dx[1] * p[0]) / det,
        ]
    });
    let lo: [u32; 2] = std::array::from_fn(|c| {
        (points
            .iter()
            .map(|p| p[c])
            .fold(f64::INFINITY, f64::min)
            .floor()
            .max(0.) as u32)
            .saturating_sub(1)
    });
    let hi: [u32; 2] = std::array::from_fn(|c| {
        ((points
            .iter()
            .map(|p| p[c])
            .fold(f64::NEG_INFINITY, f64::max)
            .ceil()
            + 1.)
            .max(0.) as u32)
            .min(output[c])
    });
    (
        lo,
        [hi[0].saturating_sub(lo[0]), hi[1].saturating_sub(lo[1])],
    )
}

#[cfg(any(test, all(feature = "gpu", feature = "render")))]
pub(crate) fn bounded(window: Window) -> Vec<Window> {
    bounded_by(window, [512, 128])
}

pub(crate) fn bounded_by((origin, extent): Window, tile: [u32; 2]) -> Vec<Window> {
    let mut result = Vec::new();
    for y in (0..extent[1]).step_by(tile[1] as usize) {
        for x in (0..extent[0]).step_by(tile[0] as usize) {
            result.push((
                [origin[0] + x, origin[1] + y],
                [tile[0].min(extent[0] - x), tile[1].min(extent[1] - y)],
            ));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magnified_support_is_bounded_without_changing_coverage_or_resolution() {
        let output = [5120, 2880];
        let full = affine_output(
            [[0., 0.], [0.001, 0.], [0., 0.001]],
            output,
            [0, 0],
            [512, 128],
        );
        assert_eq!(full, ([0, 0], output));
        let windows = bounded(full);
        assert!(windows.len() > 1);
        assert!(windows.iter().all(|(_, n)| n[0] <= 512 && n[1] <= 128));
        assert_eq!(
            windows
                .iter()
                .map(|(_, n)| u64::from(n[0]) * u64::from(n[1]))
                .sum::<u64>(),
            u64::from(output[0]) * u64::from(output[1])
        );
    }
}
