//! Bounded camera-RGB flat-field estimation and source-coordinate evaluation.
use crate::{Error, color::AP1_TO_XYZ};
use glam::{DMat3, DVec3};

pub const CONTRACT: &str = "camera-rgb-neutral-gray-local-quadratic-log-gaussian-v1";
const MAXIMUM: usize = 256;
const SIGMA: f64 = 6.;

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "project", derive(serde::Serialize))]
#[cfg_attr(feature = "project", serde(deny_unknown_fields))]
pub struct FlatField {
    area: [u32; 4],
    size: [usize; 2],
    gains: Vec<DVec3>,
}

#[cfg(feature = "project")]
impl<'de> serde::Deserialize<'de> for FlatField {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Nodes {
            area: [u32; 4],
            size: [usize; 2],
            gains: Vec<DVec3>,
        }
        let nodes = Nodes::deserialize(deserializer)?;
        Self::from_nodes(nodes.area, nodes.size, nodes.gains).map_err(serde::de::Error::custom)
    }
}

impl FlatField {
    /// Nodes include the rectangle edges; evaluation uses source pixel-centre coordinates.
    /// Outside this support the field is deliberately identity, not an extrapolated estimate.
    pub fn from_nodes(area: [u32; 4], size: [usize; 2], gains: Vec<DVec3>) -> Result<Self, Error> {
        let field = Self { area, size, gains };
        field.validate()?;
        Ok(field)
    }

    pub fn validate(&self) -> Result<(), Error> {
        let Self { area, size, gains } = self;
        check_area(*area)?;
        if size.iter().any(|v| *v < 2 || *v > MAXIMUM) || gains.len() != size[0] * size[1] {
            return Err(Error::InvalidParameter(
                "invalid flat-field grid dimensions",
            ));
        }
        if gains.iter().any(|g| {
            !g.is_finite()
                || g.min_element() <= 0.
                || g.to_array().iter().any(|v| !(*v as f32).is_normal())
        }) {
            return Err(Error::InvalidParameter(
                "flat-field gains must be positive, finite and representable",
            ));
        }
        Ok(())
    }

    /// Camera means are full-pixel means of equal-size cells entirely within `area`.
    /// `center_camera` is the separately measured image-centre patch, not a single pixel.
    pub fn estimate(
        area: [u32; 4],
        size: [usize; 2],
        camera_means: &[DVec3],
        camera_to_ap1: DMat3,
        center_camera: DVec3,
        center: [f64; 2],
    ) -> Result<Self, Error> {
        check_area(area)?;
        if size.iter().any(|v| *v < 7 || *v > MAXIMUM) || camera_means.len() != size[0] * size[1] {
            return Err(Error::InvalidParameter(
                "flat-field estimation requires a bounded grid of at least 7 by 7 cells",
            ));
        }
        if camera_means
            .iter()
            .chain(std::iter::once(&center_camera))
            .any(|p| !p.is_finite() || p.min_element() <= 0. || p.max_element() > 1.)
        {
            return Err(Error::UnusableSample(
                "flat-field camera means must be finite and within 0..1",
            ));
        }
        if !camera_to_ap1.is_finite() || camera_to_ap1.determinant().abs() < 1e-8 {
            return Err(Error::InvalidParameter("invalid flat-field camera matrix"));
        }
        if center.iter().any(|v| !v.is_finite()) || !contains(area, center) {
            return Err(Error::InvalidParameter(
                "flat-field support must contain the image centre",
            ));
        }
        let luminance = (AP1_TO_XYZ * (camera_to_ap1 * center_camera)).y;
        let target = camera_to_ap1.inverse() * DVec3::splat(luminance);
        if !target.is_finite() || target.min_element() <= 0. {
            return Err(Error::UnusableSample(
                "flat-field neutral target is not positive in camera RGB",
            ));
        }
        let [w, h] = size;
        let mut smooth = Vec::new();
        for channel in 0..3 {
            let logs: Vec<_> = camera_means.iter().map(|p| p[channel].ln()).collect();
            smooth.push(smooth_surface(&logs, w, h)?);
        }
        let mut gains = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                // Extend a fitted slope only across the half-cell inside the accepted rectangle.
                let xx = x as f64 * w as f64 / (w - 1) as f64 - 0.5;
                let yy = y as f64 * h as f64 / (h - 1) as f64 - 0.5;
                let light = DVec3::new(
                    sample(&smooth[0], w, h, xx, yy).exp(),
                    sample(&smooth[1], w, h, xx, yy).exp(),
                    sample(&smooth[2], w, h, xx, yy).exp(),
                );
                gains.push(target / light);
            }
        }
        let mut field = Self::from_nodes(area, size, gains)?;
        // Normalize the actual bilinear evaluator, not merely one estimation-grid node.
        let normalization = (target / center_camera) / field.gain(center)?;
        for g in &mut field.gains {
            *g *= normalization;
        }
        Self::from_nodes(area, size, field.gains)
    }

    pub fn area(&self) -> [u32; 4] {
        self.area
    }

    pub fn size(&self) -> [usize; 2] {
        self.size
    }

    pub fn nodes(&self) -> &[DVec3] {
        &self.gains
    }

    pub fn bounds(&self) -> (DVec3, DVec3) {
        self.gains
            .iter()
            .fold((DVec3::ONE, DVec3::ONE), |(min, max), g| {
                (min.min(*g), max.max(*g))
            })
    }

    /// One camera RGB16 code amplified by the largest gain, including identity outside support.
    /// Bilinear gain extrema occur at nodes.
    pub fn precision_floor(&self) -> Result<f64, Error> {
        let floor = self.bounds().1.max_element() / 65535.;
        if !floor.is_finite() || floor <= 0. {
            return Err(Error::NumericalOverflow);
        }
        Ok(floor)
    }

    pub fn processing_calibration(
        &self,
        mut calibration: crate::processing::Calibration,
    ) -> Result<crate::processing::Calibration, Error> {
        if calibration.input_floor != crate::processing::CAMERA_INPUT_FLOOR {
            return Err(Error::InvalidParameter(
                "flat-field calibration requires the native-code base floor",
            ));
        }
        calibration.input_floor = self.precision_floor()?;
        calibration.validate()
    }

    pub fn gain(&self, point: [f64; 2]) -> Result<DVec3, Error> {
        if point.iter().any(|v| !v.is_finite()) {
            return Err(Error::InvalidParameter("nonfinite flat-field coordinate"));
        }
        if !contains(self.area, point) {
            return Ok(DVec3::ONE);
        }
        let x = (point[0] - self.area[0] as f64) / self.area[2] as f64 * (self.size[0] - 1) as f64;
        let y = (point[1] - self.area[1] as f64) / self.area[3] as f64 * (self.size[1] - 1) as f64;
        let x0 = (x.floor() as usize).min(self.size[0] - 2);
        let y0 = (y.floor() as usize).min(self.size[1] - 2);
        let dx = x - x0 as f64;
        let dy = y - y0 as f64;
        let a = self.gains[y0 * self.size[0] + x0] * (1. - dx)
            + self.gains[y0 * self.size[0] + x0 + 1] * dx;
        let b = self.gains[(y0 + 1) * self.size[0] + x0] * (1. - dx)
            + self.gains[(y0 + 1) * self.size[0] + x0 + 1] * dx;
        Ok(a * (1. - dy) + b * dy)
    }
}

/// Propose central rectangular illumination from block luminance profiles.
/// Boundary cells are discarded; ambiguity returns no rectangle, never an assumed mask.
#[cfg(feature = "raw")]
pub(crate) fn propose_support(values: &[f64], size: [usize; 2], center: f64) -> Option<[usize; 4]> {
    let [w, h] = size;
    if w < 7
        || h < 7
        || values.len() != w * h
        || !center.is_finite()
        || center <= 0.
        || values.iter().any(|v| !v.is_finite())
    {
        return None;
    }
    let median = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    };
    let cols: Vec<_> = (0..w)
        .map(|x| median((h / 4..3 * h / 4).map(|y| values[y * w + x]).collect()))
        .collect();
    let rows: Vec<_> = (0..h)
        .map(|y| median((w / 4..3 * w / 4).map(|x| values[y * w + x]).collect()))
        .collect();
    let span = |profile: &[f64]| -> Option<(usize, usize)> {
        let middle = profile.len() / 2;
        let threshold = center * 0.25;
        if profile[middle] <= threshold {
            return None;
        }
        let mut first = middle;
        let mut end = middle + 1;
        while first > 0 && profile[first - 1] > threshold {
            first -= 1;
        }
        while end < profile.len() && profile[end] > threshold {
            end += 1;
        }
        (end > first + 4).then_some((first + 1, end - 1))
    };
    let (left, right) = span(&cols)?;
    let (top, bottom) = span(&rows)?;
    // Check rectangular occupancy; profiles alone can also describe a circular opening.
    let mut lit = 0;
    let mut count = 0;
    for y in top..bottom {
        for x in left..right {
            count += 1;
            if values[y * w + x] > center * 0.25 {
                lit += 1;
            }
        }
    }
    (lit as f64 / count as f64 >= 0.99).then_some([left, top, right, bottom])
}

fn check_area(a: [u32; 4]) -> Result<(), Error> {
    if a[2] == 0
        || a[3] == 0
        || a[0].checked_add(a[2]).is_none()
        || a[1].checked_add(a[3]).is_none()
    {
        return Err(Error::InvalidParameter("invalid flat-field rectangle"));
    }
    Ok(())
}

fn contains(a: [u32; 4], p: [f64; 2]) -> bool {
    p[0] >= a[0] as f64
        && p[1] >= a[1] as f64
        && p[0] < (a[0] + a[2]) as f64
        && p[1] < (a[1] + a[3]) as f64
}

fn solve(mut a: [[f64; 3]; 3], mut b: [f64; 3]) -> Result<[f64; 3], Error> {
    for k in 0..3 {
        let pivot = (k..3)
            .max_by(|&i, &j| a[i][k].abs().total_cmp(&a[j][k].abs()))
            .unwrap();
        if !a[pivot][k].is_finite() || a[pivot][k].abs() < 1e-14 {
            return Err(Error::UnusableSample("flat-field fit is ill-conditioned"));
        }
        a.swap(k, pivot);
        b.swap(k, pivot);
        for i in k + 1..3 {
            let f = a[i][k] / a[k][k];
            let pivot_row = a[k];
            for (value, pivot) in a[i][k + 1..].iter_mut().zip(&pivot_row[k + 1..]) {
                *value -= f * pivot;
            }
            b[i] -= f * b[k];
        }
    }
    let mut x = [0.; 3];
    for i in (0..3).rev() {
        x[i] = (b[i] - (i + 1..3).map(|j| a[i][j] * x[j]).sum::<f64>()) / a[i][i];
    }
    if x.iter().any(|v| !v.is_finite()) {
        return Err(Error::NumericalOverflow);
    }
    Ok(x)
}

fn plane(points: &[[f64; 3]]) -> Result<[f64; 3], Error> {
    let mut a = [[0.; 3]; 3];
    let mut b = [0.; 3];
    for p in points {
        let t = [1., p[0], p[1]];
        for i in 0..3 {
            b[i] += t[i] * p[2];
            for j in 0..3 {
                a[i][j] += t[i] * t[j];
            }
        }
    }
    solve(a, b)
}

fn kernel(at: usize, size: usize) -> Result<Vec<(usize, f64)>, Error> {
    let radius = (3. * SIGMA) as usize;
    let mut a = [[0.; 3]; 3];
    let mut points = Vec::new();
    for index in at.saturating_sub(radius)..(at + radius + 1).min(size) {
        let d = (index as f64 - at as f64) / SIGMA;
        let t = [1., d, d * d];
        let weight = (-0.5 * d * d).exp();
        for i in 0..3 {
            for j in 0..3 {
                a[i][j] += weight * t[i] * t[j];
            }
        }
        points.push((index, t, weight));
    }
    let intercept = solve(a, [1., 0., 0.])?;
    Ok(points
        .into_iter()
        .map(|(i, t, w)| {
            (
                i,
                w * t.iter().zip(&intercept).map(|(a, b)| a * b).sum::<f64>(),
            )
        })
        .collect())
}

fn smooth_surface(values: &[f64], w: usize, h: usize) -> Result<Vec<f64>, Error> {
    let mut repaired = values.to_vec();
    for y in 0..h {
        for x in 0..w {
            let mut neighbors = Vec::new();
            for yy in y.saturating_sub(2)..(y + 3).min(h) {
                for xx in x.saturating_sub(2)..(x + 3).min(w) {
                    neighbors.push([
                        xx as f64 - x as f64,
                        yy as f64 - y as f64,
                        values[yy * w + xx],
                    ]);
                }
            }
            let trend = plane(&neighbors)?;
            let mut residuals: Vec<_> = neighbors
                .iter()
                .map(|p| (p[2] - trend[0] - trend[1] * p[0] - trend[2] * p[1]).abs())
                .collect();
            residuals.sort_by(f64::total_cmp);
            if (values[y * w + x] - trend[0]).abs()
                > (6. * residuals[residuals.len() / 2]).max(0.006)
            {
                neighbors.retain(|p| p[0] != 0. || p[1] != 0.);
                repaired[y * w + x] = plane(&neighbors)?[0];
            }
        }
    }
    let xkernels: Vec<_> = (0..w).map(|x| kernel(x, w)).collect::<Result<_, _>>()?;
    let ykernels: Vec<_> = (0..h).map(|y| kernel(y, h)).collect::<Result<_, _>>()?;
    let mut horizontal = vec![0.; w * h];
    let mut result = vec![0.; w * h];
    for y in 0..h {
        for x in 0..w {
            horizontal[y * w + x] = xkernels[x]
                .iter()
                .map(|(xx, k)| repaired[y * w + xx] * k)
                .sum();
        }
    }
    for y in 0..h {
        for x in 0..w {
            result[y * w + x] = ykernels[y]
                .iter()
                .map(|(yy, k)| horizontal[yy * w + x] * k)
                .sum();
        }
    }
    Ok(result)
}

fn sample(values: &[f64], w: usize, h: usize, x: f64, y: f64) -> f64 {
    let x0 = (x.floor().max(0.) as usize).min(w - 2);
    let y0 = (y.floor().max(0.) as usize).min(h - 2);
    let dx = x - x0 as f64;
    let dy = y - y0 as f64;
    let a = values[y0 * w + x0] * (1. - dx) + values[y0 * w + x0 + 1] * dx;
    let b = values[(y0 + 1) * w + x0] * (1. - dx) + values[(y0 + 1) * w + x0 + 1] * dx;
    a * (1. - dy) + b * dy
}
#[cfg(test)]
#[path = "flat_field_tests.rs"]
mod tests;
