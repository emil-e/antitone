use glam::{DMat2, DMat3, DVec2, DVec3};

use crate::Error;
use crate::color::{Balance, neutral_wb};
use crate::processing::{Calibration, Inversion, RollSettings};

pub const FIT_RESIDUAL_TOLERANCE: f64 = 1e-9;
const DERIVATIVE_STEP: f64 = 1e-5;
const MIN_JACOBIAN_DETERMINANT: f64 = 1e-9;

pub fn mean(pixels: &[DVec3]) -> Result<DVec3, Error> {
    if pixels.is_empty() {
        return Err(Error::UnusableSample("empty patch"));
    }
    let mut sum = DVec3::ZERO;
    let mut compensation = DVec3::ZERO;
    for &pixel in pixels {
        if !pixel.is_finite() {
            let channel = pixel
                .to_array()
                .iter()
                .position(|v| !v.is_finite())
                .unwrap();
            return Err(Error::NonFinitePixel { channel });
        }
        let y = pixel - compensation;
        let t = sum + y;
        compensation = (t - sum) - y;
        sum = t;
    }
    let result = sum / pixels.len() as f64;
    if !result.is_finite() {
        return Err(Error::NumericalOverflow);
    }
    Ok(result)
}

pub fn weighted_mean(pixels: &[(DVec3, f64)]) -> Result<DVec3, Error> {
    weighted_mean_iter(pixels.iter().copied().map(Ok))
}
pub fn weighted_mean_iter(
    pixels: impl IntoIterator<Item = Result<(DVec3, f64), Error>>,
) -> Result<DVec3, Error> {
    let mut sum = DVec3::ZERO;
    let mut compensation = DVec3::ZERO;
    let mut total = 0.;
    let mut weight_compensation = 0.;
    for sample in pixels {
        let (pixel, weight) = sample?;
        if !pixel.is_finite() {
            let channel = pixel
                .to_array()
                .iter()
                .position(|v| !v.is_finite())
                .unwrap();
            return Err(Error::NonFinitePixel { channel });
        }
        if !weight.is_finite() || weight <= 0. {
            return Err(Error::UnusableSample("invalid coverage weight"));
        }
        let value = pixel * weight - compensation;
        let next = sum + value;
        compensation = (next - sum) - value;
        sum = next;
        let value = weight - weight_compensation;
        let next = total + value;
        weight_compensation = (next - total) - value;
        total = next;
    }
    if total == 0. {
        return Err(Error::UnusableSample("empty patch"));
    }
    let result = sum / total;
    if !result.is_finite() {
        return Err(Error::NumericalOverflow);
    }
    Ok(result)
}

pub fn photo_neutral_weighted(pixels: &[(DVec3, f64)]) -> Result<Balance, Error> {
    neutral_wb(weighted_mean(pixels)?)
}

/// Samples are positive-image AP1 working RGB before the existing photo WB.
pub fn photo_neutral(pixels_before_wb: &[DVec3]) -> Result<Balance, Error> {
    neutral_wb(mean(pixels_before_wb)?)
}

pub fn roll_neutral(
    calibration: Calibration,
    roll: RollSettings,
    scan_patch: &[DVec3],
    camera_to_ap1: DMat3,
) -> Result<Balance, Error> {
    fit_roll_neutral(
        calibration,
        roll,
        scan_patch.iter().map(|&p| (p, 1.)),
        camera_to_ap1,
    )
}

pub fn roll_neutral_weighted(
    calibration: Calibration,
    roll: RollSettings,
    scan_patch: &[(DVec3, f64)],
    camera_to_ap1: DMat3,
) -> Result<Balance, Error> {
    fit_roll_neutral(calibration, roll, scan_patch.iter().copied(), camera_to_ap1)
}
fn fit_roll_neutral(
    calibration: Calibration,
    roll: RollSettings,
    scan_patch: impl Iterator<Item = (DVec3, f64)> + Clone,
    camera_to_ap1: DMat3,
) -> Result<Balance, Error> {
    crate::processing::validate_camera_matrix(camera_to_ap1)?;
    let target = camera_to_ap1.inverse() * DVec3::ONE;
    if !target.is_finite() || target.min_element() <= 0. {
        return Err(Error::UnusableSample(
            "neutral AP1 is not positive in camera RGB",
        ));
    }
    calibration.validate()?;
    roll.channel_gammas()?;
    weighted_mean_iter(scan_patch.clone().map(Ok))?;
    let total = scan_patch.clone().map(|(_, w)| w).sum::<f64>();
    if scan_patch
        .clone()
        .any(|(p, _)| p.min_element() < calibration.input_floor)
    {
        return Err(Error::UnusableSample(
            "gamma patch contains floored scan inputs",
        ));
    }
    let residual = |coordinates: DVec2| -> Result<DVec2, Error> {
        let balance = Balance {
            warmth: coordinates.x,
            tint: coordinates.y,
        };
        let inversion = Inversion::new(calibration, RollSettings { balance, ..roll })?;
        let mut sum = DVec3::ZERO;
        let mut compensation = DVec3::ZERO;
        for (scan, weight) in scan_patch.clone() {
            let y = inversion.apply(scan)?.rgb * weight - compensation;
            let t = sum + y;
            compensation = (t - sum) - y;
            sum = t;
        }
        let positive = sum / total;
        if !positive.is_finite() || positive.min_element() <= 1e-8 {
            return Err(Error::UnusableSample(
                "gamma patch is too near Dmin or has nonpositive mean RGB",
            ));
        }
        Ok(DVec2::new(
            (positive.x / target.x).ln() - (positive.y / target.y).ln(),
            (positive.z / target.z).ln() - (positive.y / target.y).ln(),
        ))
    };

    let mut x = DVec2::new(roll.balance.warmth, roll.balance.tint);
    for _ in 0..32 {
        let r = residual(x)?;
        let mut jacobian = DMat2::ZERO;
        for axis in 0..2 {
            let mut lo = x;
            let mut hi = x;
            lo[axis] = (x[axis] - DERIVATIVE_STEP).max(-1.0);
            hi[axis] = (x[axis] + DERIVATIVE_STEP).min(1.0);
            *jacobian.col_mut(axis) = (residual(hi)? - residual(lo)?) / (hi[axis] - lo[axis]);
        }
        let det = jacobian.determinant();
        if !jacobian.is_finite() || !det.is_finite() || det.abs() < MIN_JACOBIAN_DETERMINANT {
            return Err(Error::UnusableSample(
                "gamma patch has insufficient slope information (near an anchor)",
            ));
        }
        if r.length() < FIT_RESIDUAL_TOLERANCE {
            return Ok(Balance {
                warmth: x.x,
                tint: x.y,
            });
        }
        let delta = jacobian.inverse() * r;
        let mut accepted = false;
        for attempt in 0..16 {
            let step = 0.5_f64.powi(attempt);
            let candidate = x - step * delta;
            if !candidate.is_finite() || candidate.abs().max_element() > 1.0 {
                continue;
            }
            if let Ok(candidate_residual) = residual(candidate)
                && candidate_residual.length() < r.length()
            {
                x = candidate;
                accepted = true;
                break;
            }
        }
        if !accepted {
            return Err(Error::UnusableSample(
                "no usable gamma correction within the control range",
            ));
        }
    }
    Err(Error::UnusableSample("gamma fit did not converge"))
}
