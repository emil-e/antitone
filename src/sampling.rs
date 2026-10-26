use glam::{DMat2, DMat3, DVec2, DVec3};

use crate::Error;
use crate::color::{Balance, ap1_to_cones, neutral_wb, opponent_coordinates};
use crate::processing::{Inversion, NegativeInput};

pub const FIT_RESIDUAL_TOLERANCE: f64 = 1e-9;
const DERIVATIVE_STEP: f64 = 1e-5;
const MIN_JACOBIAN_DETERMINANT: f64 = 1e-9;

pub fn mean(pixels: &[DVec3]) -> Result<DVec3, Error> {
    if pixels.is_empty() {
        return Err(Error::UnusableSample("empty patch"));
    }
    let mut sum = DVec3::ZERO;
    for &pixel in pixels {
        if !pixel.is_finite() {
            let channel = pixel
                .to_array()
                .iter()
                .position(|v| !v.is_finite())
                .unwrap();
            return Err(Error::NonFinitePixel { channel });
        }
        sum += pixel;
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
    let mut total = 0.;
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
        sum += pixel * weight;
        total += weight;
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

/// Per-channel mean of the 98–99.5% quantile band in linear camera RGB.
/// The caller supplies eligible film samples pooled across the roll; this does not
/// identify film, validate capture/camera compatibility, or establish pool adequacy.
pub fn roll_dmin(pixels: &[DVec3], input_floor: f64) -> Result<DVec3, Error> {
    if !input_floor.is_finite() || input_floor <= 0. {
        return Err(Error::InvalidParameter("invalid roll sampling floor"));
    }
    if pixels.is_empty() {
        return Err(Error::UnusableSample("empty roll film samples"));
    }
    for &pixel in pixels {
        for (channel, value) in pixel.to_array().into_iter().enumerate() {
            if !value.is_finite() {
                return Err(Error::NonFinitePixel { channel });
            }
            if value <= input_floor {
                return Err(Error::UnusableSample(
                    "roll film samples contain floored inputs",
                ));
            }
        }
    }
    if pixels.len() == 1 {
        return Ok(pixels[0]);
    }
    let start = (pixels.len() - 1) as f64 * 0.98;
    let end = (pixels.len() - 1) as f64 * 0.995;
    let mut result = DVec3::ZERO;
    for channel in 0..3 {
        let mut values: Vec<_> = pixels.iter().map(|p| p[channel]).collect();
        values.sort_unstable_by(f64::total_cmp);
        let mut sum = 0.;
        for i in start.floor() as usize..end.ceil() as usize {
            let left = start.max(i as f64);
            let right = end.min((i + 1) as f64);
            let delta = values[i + 1] - values[i];
            let a = values[i] + delta * (left - i as f64);
            let b = values[i] + delta * (right - i as f64);
            let term = (a * 0.5 + b * 0.5) * ((right - left) / (end - start));
            sum += term;
        }
        result[channel] = sum;
    }
    if !result.is_finite() {
        return Err(Error::NumericalOverflow);
    }
    Ok(result)
}

/// Fit the selected layer's WB before its contrast/saturation, using each sample's
/// effective layer weight. Samples contain exposed upstream AP1, patch area and layer weight.
pub fn adjustment_layer_white_balance(pixels: &[(DVec3, f64, f64)]) -> Result<Balance, Error> {
    weighted_mean_iter(pixels.iter().map(|&(rgb, area, _)| Ok((rgb, area))))?;
    if pixels
        .iter()
        .any(|&(_, _, weight)| !weight.is_finite() || !(0. ..=1.).contains(&weight))
    {
        return Err(Error::InvalidParameter(
            "WB layer weights must be within 0..1",
        ));
    }
    if !pixels
        .iter()
        .any(|&(_, area, weight)| area > 0. && weight > 0.)
    {
        return Err(Error::UnusableSample(
            "WB patch is outside the enabled layer's coverage",
        ));
    }
    let cones = ap1_to_cones();
    let white = cones * DVec3::ONE;
    let samples: Vec<_> = pixels
        .iter()
        .map(|&(rgb, area, weight)| (cones * rgb * area, weight))
        .collect();
    fit_balance(
        |coordinates| {
            let offsets = Balance {
                warmth: coordinates.x,
                tint: coordinates.y,
            }
            .offsets()?;
            let sample: DVec3 = samples
                .iter()
                .map(|&(rgb, weight)| rgb * (offsets * weight).exp())
                .sum();
            if !sample.is_finite() || sample.min_element() <= 0. {
                return Err(Error::UnusableSample(
                    "WB patch has invalid Bradford cone responses",
                ));
            }
            Ok(opponent_coordinates((white / sample).ln()))
        },
        Balance::default(),
        Conditioning::Relative,
    )
}

pub fn white_balance_weighted(pixels: &[(DVec3, f64)]) -> Result<Balance, Error> {
    neutral_wb(weighted_mean(pixels)?)
}

/// Samples are positive-image AP1 working RGB before the balance being fitted.
pub fn white_balance(pixels_before_wb: &[DVec3]) -> Result<Balance, Error> {
    neutral_wb(mean(pixels_before_wb)?)
}

pub fn roll_neutral(
    negative: impl Into<NegativeInput>,
    scan_patch: &[DVec3],
    camera_to_ap1: DMat3,
) -> Result<Balance, Error> {
    fit_roll_neutral(
        negative.into(),
        scan_patch.iter().map(|&p| (p, 1.)),
        camera_to_ap1,
    )
}

pub fn roll_neutral_weighted(
    negative: impl Into<NegativeInput>,
    scan_patch: &[(DVec3, f64)],
    camera_to_ap1: DMat3,
) -> Result<Balance, Error> {
    fit_roll_neutral(negative.into(), scan_patch.iter().copied(), camera_to_ap1)
}

fn fit_roll_neutral(
    negative: NegativeInput,
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
    negative.calibration.validate()?;
    negative.channel_gammas()?;
    weighted_mean_iter(scan_patch.clone().map(Ok))?;
    let total = scan_patch.clone().map(|(_, w)| w).sum::<f64>();
    if scan_patch
        .clone()
        .any(|(p, _)| p.min_element() < negative.calibration.input_floor)
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
        let inversion = Inversion::new(NegativeInput {
            balance,
            ..negative
        })?;
        let mut sum = DVec3::ZERO;
        for (scan, weight) in scan_patch.clone() {
            sum += inversion.apply(scan)?.rgb * weight;
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

    fit_balance(residual, negative.balance, Conditioning::Absolute)
}

// Gamma fitting rejects weak anchor information; layer WB can have a small,
// well-conditioned Jacobian solely because coverage attenuates every sample.
enum Conditioning {
    Absolute,
    Relative,
}

fn fit_balance(
    residual: impl Fn(DVec2) -> Result<DVec2, Error>,
    initial: Balance,
    conditioning: Conditioning,
) -> Result<Balance, Error> {
    let mut x = DVec2::new(initial.warmth, initial.tint);
    for _ in 0..32 {
        let r = residual(x)?;
        let numerical_floor = match conditioning {
            Conditioning::Absolute => 0.,
            Conditioning::Relative => 16. * f64::EPSILON,
        };
        if numerical_floor != 0. && r.length() <= numerical_floor {
            return Ok(Balance {
                warmth: x.x,
                tint: x.y,
            });
        }
        let mut jacobian = DMat2::ZERO;
        for axis in 0..2 {
            let mut lo = x;
            let mut hi = x;
            lo[axis] = (x[axis] - DERIVATIVE_STEP).max(-1.0);
            hi[axis] = (x[axis] + DERIVATIVE_STEP).min(1.0);
            *jacobian.col_mut(axis) = (residual(hi)? - residual(lo)?) / (hi[axis] - lo[axis]);
        }
        let scale = match conditioning {
            Conditioning::Absolute => 1.,
            Conditioning::Relative => jacobian
                .to_cols_array()
                .into_iter()
                .map(f64::abs)
                .fold(0., f64::max),
        };
        if !scale.is_finite() || scale == 0. {
            return Err(Error::UnusableSample(
                "neutral patch has insufficient correction information",
            ));
        }
        jacobian /= scale;
        let det = jacobian.determinant();
        if !jacobian.is_finite() || !det.is_finite() || det.abs() < MIN_JACOBIAN_DETERMINANT {
            return Err(Error::UnusableSample(
                "neutral patch has insufficient correction information",
            ));
        }
        if r.length() <= FIT_RESIDUAL_TOLERANCE * scale + numerical_floor {
            return Ok(Balance {
                warmth: x.x,
                tint: x.y,
            });
        }
        let delta = jacobian.inverse() * (r / scale);
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
                "no usable neutral correction within the control range",
            ));
        }
    }
    Err(Error::UnusableSample("neutral fit did not converge"))
}
