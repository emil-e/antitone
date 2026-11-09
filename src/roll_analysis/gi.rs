use super::*;

/// A regular raster of original corrected camera samples, not reconstructed display pixels.
/// The caller establishes spatial/material support; false entries never supply GI evidence.
#[derive(Debug)]
pub struct SpatialSamples {
    size: [usize; 2],
    camera: Vec<DVec3>,
    valid: Vec<bool>,
}

struct PositiveRaster {
    rgb: Vec<DVec3>,
    valid: Vec<bool>,
}

impl AsRef<SpatialSamples> for SpatialSamples {
    fn as_ref(&self) -> &SpatialSamples {
        self
    }
}

impl SpatialSamples {
    pub fn new(size: [usize; 2], camera: Vec<DVec3>, valid: Vec<bool>) -> Result<Self, Error> {
        if size.iter().any(|&v| v == 0 || v > GRAYNESS_LONG_EDGE)
            || camera.len() != size[0] * size[1]
            || valid.len() != camera.len()
        {
            return Err(Error::InvalidParameter("invalid GI camera raster"));
        }
        if camera
            .iter()
            .zip(&valid)
            .any(|(p, ok)| *ok && !p.is_finite())
        {
            return Err(Error::NumericalOverflow);
        }
        Ok(Self {
            size,
            camera,
            valid,
        })
    }

    pub fn size(&self) -> [usize; 2] {
        self.size
    }

    pub fn camera_samples(&self) -> &[DVec3] {
        &self.camera
    }

    pub fn eligibility(&self) -> &[bool] {
        &self.valid
    }

    fn response(
        &self,
        negative: NegativeInput,
        model: &mut impl ForwardModel,
        cancelled: &mut impl FnMut() -> bool,
    ) -> Result<Option<PositiveRaster>, Error> {
        let valid: Vec<_> = self
            .camera
            .iter()
            .zip(&self.valid)
            .map(|(p, ok)| *ok && p.min_element() > negative.calibration.input_floor)
            .collect();
        let indices: Vec<_> = (0..valid.len()).filter(|&i| valid[i]).collect();
        let points: Vec<_> = indices.iter().map(|&i| self.camera[i]).collect();
        let batch = SampleBatch::new([points.as_slice()]);
        let mut rgb = vec![DVec3::ZERO; self.camera.len()];
        if !batch.read(model, negative, cancelled, |_, i, positive| {
            rgb[indices[i]] = positive;
            Ok(())
        })? {
            return Ok(None);
        }
        Ok(Some(PositiveRaster { rgb, valid }))
    }
}

/// Both evidence populations belong to the same photograph and camera/correction basis.
#[derive(Clone, Copy)]
pub struct SpatialFrameSamples<'a> {
    pub statistics: FrameSamples<'a>,
    pub spatial: &'a SpatialSamples,
}

/// Estimate a GI-assisted starting grade from ordered sparse statistics and spatial evidence.
/// One ordinary GI gamma fit follows the point-only seed, then GI WB and shared exposure refresh.
/// No settings are installed; cancellation returns no proposal.
pub fn estimate_spatial(
    frames: &[SpatialFrameSamples<'_>],
    effective_floor: f64,
    matrix: DMat3,
    current: impl Into<InputSettings>,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Option<Proposal>, Error> {
    estimate_spatial_with_model(
        frames,
        effective_floor,
        matrix,
        current,
        &mut model::ScalarReference,
        &mut cancelled,
    )
}

pub(crate) fn estimate_spatial_with_model(
    frames: &[SpatialFrameSamples<'_>],
    effective_floor: f64,
    matrix: DMat3,
    current: impl Into<InputSettings>,
    model: &mut impl ForwardModel,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<Option<Proposal>, Error> {
    let statistics: Vec<_> = frames.iter().map(|f| f.statistics).collect();
    estimate_spatial_with_provider(
        &statistics,
        effective_floor,
        matrix,
        current,
        model,
        cancelled,
        |index, _| Ok(Some(frames[index].spatial)),
    )
}

/// Spatial camera evidence is evaluated per photograph and released between selections.
pub(crate) fn estimate_spatial_with_provider<S: AsRef<SpatialSamples>>(
    frames: &[FrameSamples<'_>],
    effective_floor: f64,
    matrix: DMat3,
    current: impl Into<InputSettings>,
    model: &mut impl ForwardModel,
    cancelled: &mut impl FnMut() -> bool,
    image: impl FnMut(usize, &mut dyn FnMut() -> bool) -> Result<Option<S>, Error>,
) -> Result<Option<Proposal>, Error> {
    let Some(seed) =
        estimate_with_model(frames, effective_floor, matrix, current, model, cancelled)?
    else {
        return Ok(None);
    };
    refine_from(
        frames,
        matrix,
        effective_floor,
        seed,
        model,
        cancelled,
        image,
    )
}

#[cfg(test)]
fn refine(
    frames: &[FrameSamples<'_>],
    images: &[&SpatialSamples],
    matrix: DMat3,
    effective_floor: f64,
    proposal: Proposal,
    model: &mut impl ForwardModel,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<Option<Proposal>, Error> {
    refine_from(
        frames,
        matrix,
        effective_floor,
        proposal,
        model,
        cancelled,
        |index, _| Ok(Some(images[index])),
    )
}

fn refine_from<S: AsRef<SpatialSamples>>(
    frames: &[FrameSamples<'_>],
    matrix: DMat3,
    effective_floor: f64,
    mut proposal: Proposal,
    model: &mut impl ForwardModel,
    cancelled: &mut impl FnMut() -> bool,
    mut image: impl FnMut(usize, &mut dyn FnMut() -> bool) -> Result<Option<S>, Error>,
) -> Result<Option<Proposal>, Error> {
    let mut initial = proposal
        .input
        .negative_input()
        .expect("negative analysis seed");
    // The proposal persists the native floor; response domains and GI admission
    // use the common effective floor established by source acquisition.
    initial.calibration.input_floor = effective_floor;
    // This threshold gain stays at the seed grade throughout the single GI cycle.
    // It is not a WB pre-correction or an exposure-normalization step.
    let gain = exposure_multiplier(proposal.roll.exposure)?;
    let mut points = Vec::with_capacity(frames.len());
    for index in 0..frames.len() {
        let Some(samples) = image(index, cancelled)? else {
            return Ok(None);
        };
        let image = samples.as_ref();
        let Some(PositiveRaster { rgb, valid }) = image.response(initial, model, cancelled)? else {
            return Ok(None);
        };
        // Native-positive GI can favour saturated colours outside the log-cone domain.
        // Establish defined evidence at the seed curves before ranking gamma samples.
        let to_cones = crate::color::ap1_to_cones() * matrix;
        let mut admitted = valid.clone();
        for (positive, valid) in rgb.iter().zip(&mut admitted) {
            if *valid {
                let cones = to_cones * *positive;
                if !cones.is_finite() {
                    return Err(Error::NumericalOverflow);
                }
                *valid = cones.min_element() > 0.;
            }
        }
        let Some(selected) =
            grayness::select_admitted(&rgb, &valid, &admitted, image.size, gain, cancelled)
                .map_err(|error| {
                    Error::Project(format!("photo {} GI gamma evidence: {error}", index + 1))
                })?
        else {
            return Ok(None);
        };
        if selected.indices.len() < 10 {
            return Err(Error::Project(format!(
                "photo {} has too few GI gamma samples",
                index + 1
            )));
        }
        points.push(
            selected
                .indices
                .into_iter()
                .map(|i| image.camera[i])
                .collect::<Vec<_>>(),
        );
    }
    let Some(balance) = gamma::fit_with_model(initial, matrix, &points, model, cancelled)? else {
        return Ok(None);
    };
    let negative = NegativeInput { balance, ..initial };
    let mut corrections = Vec::new();
    for (index, selected_gamma) in points.iter().enumerate() {
        let Some(samples) = image(index, cancelled)? else {
            return Ok(None);
        };
        let image = samples.as_ref();
        let Some(PositiveRaster { rgb, valid }) = image.response(negative, model, cancelled)?
        else {
            return Ok(None);
        };
        let (count, correction) = match grayness::select(&rgb, &valid, image.size, gain, cancelled)
        {
            Ok(Some(selected)) => (selected.indices.len(), neutral_wb(matrix * selected.mean)),
            Ok(None) => return Ok(None),
            Err(error @ Error::UnusableSample(_)) => (0, Err(error)),
            Err(error) => return Err(error),
        };
        let rejection = match correction {
            Ok(wb) => {
                corrections.push(wb);
                None
            }
            Err(error @ Error::UnusableSample(_)) => Some(error.to_string()),
            Err(error) => return Err(error),
        };
        proposal.frames[index].grayness = Some(GraynessStatistics {
            gamma_samples: selected_gamma.len(),
            wb_samples: count,
            wb_rejection: rejection,
        });
    }
    if corrections.is_empty() {
        return Err(Error::UnusableSample(
            "no admitted GI white-balance estimates",
        ));
    }
    let mut warmth: Vec<_> = corrections.iter().map(|p| p.warmth).collect();
    let mut tint: Vec<_> = corrections.iter().map(|p| p.tint).collect();
    proposal.roll.wb = Balance {
        warmth: quantile(&mut warmth, 0.5),
        tint: quantile(&mut tint, 0.5),
    };
    let picture = SampleBatch::new(frames.iter().map(|f| f.picture));
    let Some((ev, counts)) = meter(
        &picture,
        negative,
        matrix,
        proposal.roll.wb,
        model,
        cancelled,
    )?
    else {
        return Ok(None);
    };
    proposal.roll.exposure = ev;
    for (statistics, count) in proposal.frames.iter_mut().zip(counts) {
        statistics.exposure_samples = count;
    }
    if let InputSettings::Negative { balance, .. } = &mut proposal.input {
        *balance = negative.balance;
    }
    Ok((!cancelled()).then_some(proposal))
}

#[cfg(test)]
mod tests;
