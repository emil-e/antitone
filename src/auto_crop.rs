//! Framing proposals from retained source evidence, never from statistical sampling coverage.
use crate::{
    Error,
    crop::Crop,
    geometry::SourceRegion,
    scan_regions::{Areas, Quad},
};

const MAX_EDGE_RESIDUAL: f64 = 2.;
pub const DEFAULT_MARGIN_PERCENT: f64 = 0.5;
pub const MAX_MARGIN_PERCENT: f64 = 5.;

/// Equal inward distance on every edge, as a percentage of the original frame's short side.
/// Always start from the original proposal; zero restores it exactly. This never changes sampling.
pub fn inward_margin(proposal: Crop, active: SourceRegion, percent: f64) -> Result<Crop, Error> {
    proposal.region(active)?;
    if !percent.is_finite() || !(0.0..=MAX_MARGIN_PERCENT).contains(&percent) {
        return Err(Error::InvalidParameter(
            "invalid auto-crop margin percentage",
        ));
    }
    let short = proposal.size[0].min(proposal.size[1]);
    let distance = f64::from(short) * percent / 100.;
    // Integer zero-angle edges require symmetric whole-pixel trims. Rotated frames
    // can retain their centre with odd extents too. Tiny frames stop before becoming empty.
    let removed = if proposal.angle == 0. {
        2 * (distance.ceil() as u32).min((short - 1) / 2)
    } else {
        ((2. * distance).ceil() as u32).min(short - 1)
    };
    let crop = Crop {
        size: proposal.size.map(|n| n - removed),
        ..proposal
    };
    crop.region(active)?;
    Ok(crop)
}

/// Propose crop and straightening together. Incomplete/weak evidence cannot certify a frame.
/// The caller owns availability, source freshness and promotion; this never edits the evidence.
pub fn propose(active: SourceRegion, areas: &Areas) -> Result<Crop, Error> {
    areas.validate(active)?;
    if areas.picture_edited || !areas.picture_observed.iter().all(|&v| v) {
        return Err(Error::UnusableSample(
            "four measured picture edges are required",
        ));
    }
    let geometry = areas
        .picture_geometry
        .as_ref()
        .ok_or(Error::UnusableSample(
            "retained picture measurements are missing",
        ))?;
    let quad = geometry.frame(active).ok_or(Error::UnusableSample(
        "picture measurements do not enclose a frame",
    ))?;
    Areas::coverage(quad)?
        .bounds(active)
        .map_err(|_| Error::UnusableSample("measured picture frame extends beyond the source"))?;
    let scale = [
        f64::from(active.width) / f64::from(geometry.size[0]),
        f64::from(active.height) / f64::from(geometry.size[1]),
    ];
    // The detector's common angle is quantized; allow a small capture-pixel residual.
    // Larger departures abstain rather than forcing conflicting edges into a rectangle.
    for &[a, b, c] in &areas.picture_constraints {
        let normal = (a * scale[0]).hypot(b * scale[1]);
        if quad
            .iter()
            .any(|p| (c - a * p[0] - b * p[1]) / normal > MAX_EDGE_RESIDUAL)
        {
            return Err(Error::UnusableSample(
                "measured picture edges are not rectangular",
            ));
        }
    }
    rectangle(active, quad)
}

fn rectangle(active: SourceRegion, q: Quad) -> Result<Crop, Error> {
    let x = [q[1][0] - q[0][0], q[1][1] - q[0][1]];
    let y = [q[3][0] - q[0][0], q[3][1] - q[0][1]];
    let width = x[0].hypot(x[1]);
    let u = x.map(|v| v / width);
    let height = u[0] * y[1] - u[1] * y[0];
    let skew = u[0] * y[0] + u[1] * y[1];
    // Rounded analysis raster dimensions can map its rectangle to a parallelogram.
    // Inscribe about the measured centre instead of pretending those axes are orthogonal.
    let extent = [width - skew.abs(), height];
    if extent
        .iter()
        .any(|v| !v.is_finite() || *v < 1. || *v > f64::from(u32::MAX))
    {
        return Err(Error::UnusableSample("measured frame is too small"));
    }
    let center = [(q[0][0] + q[2][0]) * 0.5, (q[0][1] + q[2][1]) * 0.5];
    let angle = x[1].atan2(x[0]);
    let crop = if angle == 0. {
        let min = q[0].map(f64::ceil);
        let max = q[2].map(f64::floor);
        if (0..2).any(|i| max[i] - min[i] < 1.) {
            return Err(Error::UnusableSample(
                "no full source pixel fits the measured frame",
            ));
        }
        Crop {
            center: [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5],
            size: [(max[0] - min[0]) as u32, (max[1] - min[1]) as u32],
            angle: 0.,
        }
    } else {
        Crop {
            center,
            size: extent.map(|v| v.floor() as u32),
            angle,
        }
    };
    crop.region(active)?;
    Ok(crop)
}

#[cfg(test)]
#[path = "auto_crop_tests.rs"]
mod tests;
