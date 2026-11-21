//! Channel balance from the same roll calibration that anchors inversion.
use super::*;
use crate::{color::opponent_coordinates, processing::log_ratio};

pub(crate) fn balance_from_anchors(calibration: Calibration) -> Option<Balance> {
    let spans = DVec3::from_array(std::array::from_fn(|c| {
        log_ratio(calibration.dmin[c], calibration.dmax[c])
    }));
    if !spans.is_finite() || spans.min_element() <= 1e-6 {
        return None;
    }
    let coordinates = opponent_coordinates(spans.ln());
    Balance {
        warmth: coordinates.x,
        tint: coordinates.y,
    }
    .validate()
    .ok()
}

pub(crate) fn estimate_anchor_spans(
    frames: &[FrameSamples<'_>],
    effective_floor: f64,
    matrix: DMat3,
    input: InputSettings,
    model: &mut impl ForwardModel,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<Option<Proposal>, Error> {
    if cancelled() {
        return Ok(None);
    }
    validate_camera_matrix(matrix)?;
    let InputSettings::Negative {
        response,
        gamma,
        balance,
        ..
    } = input
    else {
        return Err(Error::InvalidParameter(
            "roll analysis requires a negative input",
        ));
    };
    let Some(calibration) = calibrate(frames, effective_floor, cancelled)? else {
        return Ok(None);
    };
    let negative = NegativeInput {
        calibration,
        response,
        gamma,
        balance: balance_from_anchors(calibration).unwrap_or(balance),
    };
    negative.channel_gammas()?;
    let picture = SampleBatch::new(frames.iter().map(|frame| frame.picture));
    let Some((wb, _)) = median_wb::estimate(&picture, negative, matrix, model, cancelled)? else {
        return Ok(None);
    };
    let Some((exposure, exposure_counts)) =
        meter(&picture, negative, matrix, wb, model, cancelled)?
    else {
        return Ok(None);
    };
    let anchor_count = frames.iter().map(|f| f.picture.len()).min().unwrap();
    let statistics = frames
        .iter()
        .zip(exposure_counts)
        .map(|(frame, exposure_samples)| FrameStatistics {
            film_samples: frame.film.len(),
            picture_samples: frame.picture.len(),
            balance_eligible: frame.picture.len(),
            balance_samples: anchor_count,
            exposure_samples,
            grayness: None,
        })
        .collect();
    Ok((!cancelled()).then(|| Proposal {
        input: InputSettings::Negative {
            dmin: Some(calibration.dmin),
            dmax: Some(calibration.dmax),
            response,
            gamma,
            balance: negative.balance,
        },
        roll: RollSettings { wb, exposure },
        photos: vec![AdjustmentLayerSettings::default(); frames.len()],
        frames: statistics,
    }))
}

#[cfg(test)]
mod tests;
