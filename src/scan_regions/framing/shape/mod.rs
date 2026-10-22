//! One continuous ordered ratio per roll; inferred draft edges never become measured evidence.
#[cfg(test)]
use crate::{geometry::SourceRegion, scan_regions::PictureGeometry};
use serde::Serialize;

#[cfg(test)]
mod tests;

type Result<T> = std::result::Result<T, &'static str>;

#[derive(Clone, Copy)]
struct Donor {
    source: usize,
    ratio: f64,
}

#[derive(Serialize)]
pub(super) struct Shape {
    pub(super) ratio: f64,
    pub(super) interval: [f64; 2],
    pub(super) donors: Vec<usize>,
    pub(super) rejected: Vec<usize>,
}

#[derive(Serialize)]
#[cfg(test)]
pub(super) struct Draft {
    pub(super) geometry: PictureGeometry,
    pub(super) measured: [bool; 4],
    pub(super) inferred: [bool; 4],
    pub(super) inferred_band_raster: [f64; 2],
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    (values[(values.len() - 1) / 2] + values[values.len() / 2]) / 2.
}

fn fit(donors: &[Donor], target: usize) -> Result<Shape> {
    let mut identities = std::collections::HashSet::new();
    let mut peers = vec![];
    for donor in donors {
        if !identities.insert(donor.source) {
            return Err("duplicate donor identity");
        }
        if donor.source == target {
            continue;
        }
        if !donor.ratio.is_finite() || donor.ratio <= 0. {
            return Err("invalid donor ratio");
        }
        peers.push((donor.source, donor.ratio.ln()));
    }
    if peers.len() < 3 {
        return Err("fewer than three other complete donors");
    }
    let center = median(peers.iter().map(|&(_, value)| value).collect());
    let tolerance = 1.02f64.ln();
    let inliers: Vec<_> = peers
        .iter()
        .filter(|(_, value)| (*value - center).abs() <= tolerance)
        .collect();
    if inliers.len() < 3 || inliers.len() * 3 < peers.len() * 2 {
        return Err("no single reliable roll shape");
    }
    let center = median(inliers.iter().map(|&&(_, value)| value).collect());
    let mad = 1.4826
        * median(
            inliers
                .iter()
                .map(|&&(_, value)| (value - center).abs())
                .collect(),
        );
    let radius = (3. * mad).max(0.002);
    if radius > tolerance {
        return Err("roll ratio dispersion exceeds allowance");
    }
    Ok(Shape {
        ratio: center.exp(),
        interval: [(center - radius).exp(), (center + radius).exp()],
        donors: inliers.iter().map(|&&(source, _)| source).collect(),
        rejected: peers
            .iter()
            .filter(|(source, _)| !inliers.iter().any(|&&(id, _)| id == *source))
            .map(|&(source, _)| source)
            .collect(),
    })
}

#[cfg(feature = "project")]
pub(super) fn reference_fit(
    values: &[(usize, f64)],
    target: usize,
) -> std::result::Result<Shape, &'static str> {
    let donors: Vec<_> = values
        .iter()
        .map(|&(source, ratio)| Donor { source, ratio })
        .collect();
    fit(&donors, target)
}

#[cfg(test)]
fn scales(geometry: &PictureGeometry, active: SourceRegion) -> Result<[f64; 2]> {
    active.validate().map_err(|_| "invalid source area")?;
    if geometry.size.contains(&0)
        || !geometry.angle.is_finite()
        || geometry.angle.abs() > 6.
        || geometry
            .sides
            .iter()
            .flatten()
            .any(|value| !value.is_finite())
    {
        return Err("invalid frame geometry");
    }
    let scale = [
        f64::from(active.width) / f64::from(geometry.size[0]),
        f64::from(active.height) / f64::from(geometry.size[1]),
    ];
    let (s, c) = geometry.angle.to_radians().sin_cos();
    Ok([
        (scale[0] * c).hypot(scale[1] * s),
        (scale[0] * s).hypot(scale[1] * c),
    ])
}

#[cfg(test)]
fn ratio(geometry: &PictureGeometry, active: SourceRegion) -> Result<f64> {
    let [Some(left), Some(right), Some(top), Some(bottom)] = geometry.sides else {
        return Err("incomplete donor");
    };
    let [width_scale, height_scale] = scales(geometry, active)?;
    if right <= left || bottom <= top {
        return Err("empty donor geometry");
    }
    Ok((right - left) * width_scale / ((bottom - top) * height_scale))
}

#[cfg(test)]
pub(super) fn complete(
    geometry: &PictureGeometry,
    active: SourceRegion,
    shape: &Shape,
) -> Result<Draft> {
    let measured = geometry.sides.map(|value| value.is_some());
    if measured.iter().filter(|&&value| value).count() != 3 {
        return Err("exactly three measured anchors required");
    }
    let [width_scale, height_scale] = scales(geometry, active)?;
    let missing = measured.iter().position(|&value| !value).unwrap();
    let fill = |source_ratio: f64| {
        let pixel_ratio = source_ratio * height_scale / width_scale;
        let [left, right, top, bottom] = geometry.sides;
        match missing {
            0 => right.unwrap() - (bottom.unwrap() - top.unwrap()) * pixel_ratio,
            1 => left.unwrap() + (bottom.unwrap() - top.unwrap()) * pixel_ratio,
            2 => bottom.unwrap() - (right.unwrap() - left.unwrap()) / pixel_ratio,
            _ => top.unwrap() + (right.unwrap() - left.unwrap()) / pixel_ratio,
        }
    };
    let mut inferred_geometry = geometry.clone();
    inferred_geometry.sides[missing] = Some(fill(shape.ratio));
    ratio(&inferred_geometry, active)?;
    let a = fill(shape.interval[0]);
    let b = fill(shape.interval[1]);
    Ok(Draft {
        geometry: inferred_geometry,
        measured,
        inferred: std::array::from_fn(|side| side == missing),
        inferred_band_raster: [a.min(b), a.max(b)],
    })
}
