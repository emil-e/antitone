use glam::{DMat3, DVec3};

use crate::{
    Error,
    color::{AP1_TO_XYZ, Balance, neutral_wb, wb_matrix},
    processing::{
        Calibration, InputSettings, Inversion, NegativeInput, PhotoSettings, RollSettings,
        exposure_multiplier, validate_camera_matrix,
    },
    sampling,
};

#[path = "roll_analysis_gamma.rs"]
mod gamma;

pub const MAX_SAMPLES_PER_FRAME: usize = 8192;
const BALANCE_SAMPLES_PER_FRAME: usize = 2048;

/// Original, eligible camera pixels in a common development/correction basis.
/// Film may be empty for an individual frame; picture statistics may not.
#[derive(Debug, Clone, Copy)]
pub struct FrameSamples<'a> {
    pub film: &'a [DVec3],
    pub picture: &'a [DVec3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct FrameStatistics {
    pub film_samples: usize,
    pub picture_samples: usize,
    pub balance_eligible: usize,
    pub balance_samples: usize,
    pub exposure_samples: usize,
}

/// A transient proposal, not an installed grade. References are RGB-only; the
/// caller retains the native stored floor and preflights the effective correction.
#[derive(Debug, Clone, PartialEq)]
pub struct Proposal {
    pub input: InputSettings,
    pub roll: RollSettings,
    pub photos: Vec<PhotoSettings>,
    pub frames: Vec<FrameStatistics>,
}

fn check_samples(pixels: &[DVec3], floor: f64) -> Result<(), Error> {
    if pixels.len() > MAX_SAMPLES_PER_FRAME {
        return Err(Error::InvalidParameter(
            "roll analysis sample budget exceeded",
        ));
    }
    for p in pixels {
        for (channel, value) in p.to_array().into_iter().enumerate() {
            if !value.is_finite() {
                return Err(Error::NonFinitePixel { channel });
            }
            if value <= floor {
                return Err(Error::UnusableSample(
                    "roll analysis contains floored inputs",
                ));
            }
        }
    }
    Ok(())
}

fn quantile(values: &mut [f64], fraction: f64) -> f64 {
    values.sort_unstable_by(f64::total_cmp);
    let at = (values.len() - 1) as f64 * fraction;
    let low = at.floor() as usize;
    let high = at.ceil() as usize;
    // Weighted form avoids overflowing a finite median's intermediate sum.
    values[low] * (1. - (at - low as f64)) + values[high] * (at - low as f64)
}

fn centre_sample(pixels: &[DVec3], count: usize, i: usize) -> DVec3 {
    pixels[(2 * i + 1) * pixels.len() / (2 * count)]
}

/// Estimate an initial roll grade without changing caller state. Capture,
/// material, geometry and development compatibility remain sampling obligations.
/// Cancellation returns no proposal; every frame must supply picture statistics.
pub fn estimate(
    frames: &[FrameSamples<'_>],
    effective_floor: f64,
    camera_to_ap1: DMat3,
    current: impl Into<InputSettings>,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Option<Proposal>, Error> {
    if cancelled() {
        return Ok(None);
    }
    validate_camera_matrix(camera_to_ap1)?;
    if !effective_floor.is_finite() || effective_floor <= 0. {
        return Err(Error::InvalidParameter("invalid roll analysis floor"));
    }
    let InputSettings::Negative {
        input_floor,
        response,
        gamma,
        balance,
        ..
    } = current.into()
    else {
        return Err(Error::InvalidParameter(
            "roll analysis requires a negative input",
        ));
    };
    if frames.is_empty() {
        return Err(Error::UnusableSample("empty analysis roll"));
    }
    let mut film = Vec::new();
    for frame in frames {
        if cancelled() {
            return Ok(None);
        }
        check_samples(frame.film, effective_floor)?;
        check_samples(frame.picture, effective_floor)?;
        if frame.picture.is_empty() {
            return Err(Error::UnusableSample(
                "analysis frame has no picture samples",
            ));
        }
        film.extend_from_slice(frame.film);
    }
    let dmin = sampling::roll_dmin(&film, effective_floor)?;
    drop(film);
    if cancelled() {
        return Ok(None);
    }
    // Equal frame influence, using original samples rather than averaged negatives.
    let endpoint_count = frames.iter().map(|f| f.picture.len()).min().unwrap();
    let picture: Vec<_> = frames
        .iter()
        .flat_map(|f| (0..endpoint_count).map(move |i| centre_sample(f.picture, endpoint_count, i)))
        .collect();
    let dmax = DVec3::from_array(std::array::from_fn(|c| {
        let mut values: Vec<_> = picture.iter().map(|p| p[c]).collect();
        quantile(&mut values, 0.005)
    }));
    drop(picture);
    let calibration = Calibration {
        dmin,
        dmax,
        input_floor: effective_floor,
    }
    .validate()?;
    let current = NegativeInput {
        calibration,
        response,
        gamma,
        balance,
    };
    current.channel_gammas()?;
    let initial = NegativeInput {
        balance: Balance::default(),
        ..current
    };
    let baseline = Inversion::new(initial)?;
    let mut eligible = Vec::with_capacity(frames.len());
    for frame in frames {
        if cancelled() {
            return Ok(None);
        }
        let mut pixels = Vec::new();
        for &p in frame.picture {
            let v = baseline.apply(p)?;
            let y = (AP1_TO_XYZ * (camera_to_ap1 * v.rgb)).y;
            if !y.is_finite() {
                return Err(Error::NumericalOverflow);
            }
            if v.rgb.min_element() > 1e-8 && y > 1e-8 {
                pixels.push(p);
            }
        }
        if pixels.is_empty() {
            return Err(Error::UnusableSample(
                "analysis frame has no positive balance samples",
            ));
        }
        eligible.push(pixels);
    }
    let balance_count = eligible
        .iter()
        .map(Vec::len)
        .min()
        .unwrap()
        .min(BALANCE_SAMPLES_PER_FRAME);
    let selected: Vec<Vec<_>> = eligible
        .iter()
        .map(|p| {
            (0..balance_count)
                .map(|i| centre_sample(p, balance_count, i))
                .collect()
        })
        .collect();
    let Some(balance) = gamma::fit(
        NegativeInput {
            calibration,
            ..current
        },
        camera_to_ap1,
        &selected,
        &mut cancelled,
    )?
    else {
        return Ok(None);
    };
    let negative = NegativeInput {
        calibration,
        balance,
        ..initial
    };
    let inversion = Inversion::new(negative)?;
    let mut means = Vec::with_capacity(selected.len());
    for frame in &selected {
        if cancelled() {
            return Ok(None);
        }
        means.push(
            camera_to_ap1
                * sampling::weighted_mean_iter(
                    frame
                        .iter()
                        .map(|&p| inversion.apply(p).map(|v| (v.rgb, 1.))),
                )?,
        );
    }
    let mut roll = RollSettings {
        wb: neutral_wb(sampling::mean(&means)?)?,
        ..RollSettings::default()
    };
    let post = wb_matrix(roll.wb)? * camera_to_ap1;
    let mut totals = Vec::with_capacity(frames.len());
    let mut statistics = Vec::with_capacity(frames.len());
    for (index, frame) in frames.iter().enumerate() {
        if cancelled() {
            return Ok(None);
        }
        let mut luminances = Vec::new();
        for &p in frame.picture {
            let v = inversion.apply(p)?;
            let y = (AP1_TO_XYZ * (post * v.rgb)).y;
            if !y.is_finite() {
                return Err(Error::NumericalOverflow);
            }
            if y > 1e-8 {
                luminances.push(y);
            }
        }
        if luminances.is_empty() {
            return Err(Error::UnusableSample(
                "analysis frame has no positive exposure samples",
            ));
        }
        statistics.push(FrameStatistics {
            film_samples: frame.film.len(),
            picture_samples: frame.picture.len(),
            balance_eligible: eligible[index].len(),
            balance_samples: balance_count,
            exposure_samples: luminances.len(),
        });
        let p99 = quantile(&mut luminances, 0.99);
        // Cap the statistic only: signed/HDR working pixels remain unchanged.
        let capped_mean = luminances
            .iter()
            .map(|y| y.min(p99) / luminances.len() as f64)
            .sum::<f64>();
        let total = (0.18_f64.log2() - capped_mean.log2() - p99.log2()) * 0.5;
        if !total.is_finite() {
            return Err(Error::NumericalOverflow);
        }
        totals.push(total);
    }
    roll.exposure = quantile(&mut totals.clone(), 0.5);
    exposure_multiplier(roll.exposure)?;
    let photos = totals
        .into_iter()
        .map(|total| {
            let photo = PhotoSettings {
                exposure: total - roll.exposure,
                wb: Balance::default(),
                ..PhotoSettings::default()
            };
            exposure_multiplier(photo.exposure)?;
            Ok(photo)
        })
        .collect::<Result<Vec<_>, Error>>()?;
    if cancelled() {
        return Ok(None);
    }
    Ok(Some(Proposal {
        input: InputSettings::Negative {
            dmin: Some(negative.calibration.dmin),
            dmax: Some(negative.calibration.dmax),
            input_floor,
            response: negative.response,
            gamma: negative.gamma,
            balance: negative.balance,
        },
        roll,
        photos,
        frames: statistics,
    }))
}
