use crate::adjustment_layers::AdjustmentLayerSettings;
use glam::{DMat3, DVec3};

use crate::{
    Error,
    color::{AP1_TO_XYZ, Balance, neutral_wb, wb_matrix},
    processing::{
        Calibration, InputSettings, Inversion, NegativeInput, RollSettings, exposure_multiplier,
        validate_camera_matrix,
    },
    sampling,
};

mod gamma;
mod gi;
mod grayness;
mod model;

#[cfg(feature = "editor")]
pub(crate) use gi::estimate_spatial_with_provider;
pub use gi::{SpatialFrameSamples, SpatialSamples, estimate_spatial};

/// Current spatial evidence resolution, independent of output/rendering resolution.
pub const GRAYNESS_LONG_EDGE: usize = 512;

pub(crate) use model::ForwardModel;
use model::SampleBatch;

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
    pub grayness: Option<GraynessStatistics>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraynessStatistics {
    pub gamma_samples: usize,
    pub wb_samples: usize,
    pub wb_rejection: Option<String>,
}

/// A transient proposal, not an installed grade. References are RGB-only; the
/// caller retains the native stored floor and preflights the effective correction.
#[derive(Debug, Clone, PartialEq)]
pub struct Proposal {
    pub input: InputSettings,
    pub roll: RollSettings,
    pub photos: Vec<AdjustmentLayerSettings>,
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
    estimate_with_model(
        frames,
        effective_floor,
        camera_to_ap1,
        current,
        &mut model::ScalarReference,
        &mut cancelled,
    )
}

pub(crate) fn estimate_with_model(
    frames: &[FrameSamples<'_>],
    effective_floor: f64,
    camera_to_ap1: DMat3,
    current: impl Into<InputSettings>,
    model: &mut impl ForwardModel,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<Option<Proposal>, Error> {
    if cancelled() {
        return Ok(None);
    }
    validate_camera_matrix(camera_to_ap1)?;
    if !effective_floor.is_finite() || effective_floor <= 0. {
        return Err(Error::InvalidParameter("invalid roll analysis floor"));
    }
    let InputSettings::Negative {
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
    let calibration = Calibration { dmin, dmax }.validate()?;
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
    let picture_batch = SampleBatch::new(frames.iter().map(|frame| frame.picture));
    let mut eligible: Vec<Vec<DVec3>> = frames.iter().map(|_| Vec::new()).collect();
    if !picture_batch.read(model, initial, cancelled, |frame, sample, positive| {
        let y = (AP1_TO_XYZ * (camera_to_ap1 * positive)).y;
        if !y.is_finite() {
            return Err(Error::NumericalOverflow);
        }
        if positive.min_element() > 1e-8 && y > 1e-8 {
            eligible[frame].push(frames[frame].picture[sample]);
        }
        Ok(())
    })? {
        return Ok(None);
    }
    if eligible.iter().any(Vec::is_empty) {
        return Err(Error::UnusableSample(
            "analysis frame has no positive balance samples",
        ));
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
    let Some(balance) = gamma::fit_with_model(
        NegativeInput {
            calibration,
            ..current
        },
        camera_to_ap1,
        &selected,
        model,
        cancelled,
    )?
    else {
        return Ok(None);
    };
    let negative = NegativeInput {
        calibration,
        balance,
        ..initial
    };
    let selected_batch = SampleBatch::new(selected.iter().map(Vec::as_slice));
    let mut means = vec![DVec3::ZERO; selected.len()];
    if !selected_batch.read(model, negative, cancelled, |frame, _, positive| {
        means[frame] += positive;
        Ok(())
    })? {
        return Ok(None);
    }
    for (mean, group) in means.iter_mut().zip(&selected_batch.groups) {
        *mean /= group.len() as f64;
        if !mean.is_finite() {
            return Err(Error::NumericalOverflow);
        }
        *mean = camera_to_ap1 * *mean;
    }
    let mut roll = RollSettings {
        wb: neutral_wb(sampling::mean(&means)?)?,
        ..RollSettings::default()
    };
    let Some((exposure, exposure_counts)) = meter(
        &picture_batch,
        negative,
        camera_to_ap1,
        roll.wb,
        model,
        cancelled,
    )?
    else {
        return Ok(None);
    };

    let mut statistics = Vec::with_capacity(frames.len());
    for (index, frame) in frames.iter().enumerate() {
        if cancelled() {
            return Ok(None);
        }
        statistics.push(FrameStatistics {
            film_samples: frame.film.len(),
            picture_samples: frame.picture.len(),
            balance_eligible: eligible[index].len(),
            balance_samples: balance_count,
            exposure_samples: exposure_counts[index],
            grayness: None,
        });
    }
    roll.exposure = exposure;
    let photos = vec![AdjustmentLayerSettings::default(); frames.len()];
    if cancelled() {
        return Ok(None);
    }
    Ok(Some(Proposal {
        input: InputSettings::Negative {
            dmin: Some(negative.calibration.dmin),
            dmax: Some(negative.calibration.dmax),
            response: negative.response,
            gamma: negative.gamma,
            balance: negative.balance,
        },
        roll,
        photos,
        frames: statistics,
    }))
}

fn meter(
    picture: &SampleBatch,
    negative: NegativeInput,
    matrix: DMat3,
    wb: Balance,
    model: &mut impl ForwardModel,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<Option<(f64, Vec<usize>)>, Error> {
    let post = wb_matrix(wb)? * matrix;
    let mut luminances = vec![Vec::new(); picture.groups.len()];
    if !picture.read(model, negative, cancelled, |frame, _, positive| {
        let y = (AP1_TO_XYZ * (post * positive)).y;
        if !y.is_finite() {
            return Err(Error::NumericalOverflow);
        }
        if y > 1e-8 {
            luminances[frame].push(y);
        }
        Ok(())
    })? {
        return Ok(None);
    }
    let mut medians = Vec::with_capacity(luminances.len());
    let mut counts = Vec::with_capacity(luminances.len());
    for values in &mut luminances {
        if cancelled() {
            return Ok(None);
        }
        if values.is_empty() {
            return Err(Error::UnusableSample(
                "analysis frame has no positive exposure samples",
            ));
        }
        counts.push(values.len());
        medians.push(quantile(values, 0.5));
    }
    let key = quantile(&mut medians, 0.5);
    let ev = 0.18_f64.log2() - key.log2();
    exposure_multiplier(ev)?;
    Ok(Some((ev, counts)))
}
