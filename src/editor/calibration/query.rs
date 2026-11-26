//! Calibration queries use corrected camera light on the original sample grid.

use super::*;
use crate::photographic::stages::flat_field::GpuFlatField;
use crate::{
    Error,
    coverage::Coverage,
    editor::{measurement::query::Samples, white_balance::roll_from_samples},
    image_recipe::ImageRecipe,
    processing::{Calibration, log_ratio},
    roll_analysis::balance_from_anchors,
    scan::DevelopedSource,
};
use std::sync::Arc;

pub(super) fn reference(
    gpu: super::super::measurement::Gpu<'_>,
    source: &Arc<DevelopedSource>,
    field: Option<Arc<GpuFlatField>>,
    coverage: Coverage,
    cancelled: &dyn Fn() -> bool,
) -> Result<DVec3, Error> {
    let samples = Samples::new(gpu, source, field, coverage, cancelled)?;
    let mean = samples.mean(samples.camera_light()?, cancelled)?;
    if mean.min_element() <= 0. {
        return Err(Error::UnusableSample(
            "reference mean must be positive in every channel",
        ));
    }

    Ok(mean)
}

pub(super) struct RollBalancePick {
    pub gamma: Balance,
    pub wb: Balance,
    pub inferred_dmax: Option<DVec3>,
}

pub(super) fn roll_balance(
    gpu: super::super::measurement::Gpu<'_>,
    source: &Arc<DevelopedSource>,
    field: Option<Arc<GpuFlatField>>,
    coverage: Coverage,
    mut recipe: ImageRecipe,
    cancelled: &dyn Fn() -> bool,
) -> Result<RollBalancePick, Error> {
    if cancelled() {
        return Err(Error::Cancelled);
    }
    let calibration = recipe
        .neutral
        .input
        .calibration()
        .ok_or(Error::InvalidParameter(
            "measure Dmin and Dmax before balancing the roll",
        ))?
        .validate()?;
    recipe.neutral.require_source(source)?;
    let samples = Samples::new(gpu, source, field, coverage, cancelled)?;

    let patch = samples.mean(samples.camera_light()?, cancelled)?;
    let gamma = span_balance(calibration.dmin, patch)?;
    let InputSettings::Negative { balance, .. } = &mut recipe.neutral.input else {
        unreachable!("only a negative has calibration")
    };
    *balance = gamma;
    let wb = roll_from_samples(&samples, &recipe, cancelled)?;
    if cancelled() {
        return Err(Error::Cancelled);
    }

    Ok(RollBalancePick {
        gamma,
        wb,
        inferred_dmax: inferred_dmax(calibration.dmin, calibration.dmax, gamma),
    })
}

fn span_balance(dmin: DVec3, patch: DVec3) -> Result<Balance, Error> {
    // The patch supplies a direction from film base, not a replacement white anchor.
    let reference = Calibration { dmin, dmax: patch }.validate().map_err(|_| {
        Error::UnusableSample("choose a patch distinctly denser than film base in every channel")
    })?;
    balance_from_anchors(reference).ok_or(Error::UnusableSample(
        "patch implies a gamma balance outside the supported range",
    ))
}

pub(super) fn inferred_dmax(dmin: DVec3, dmax: DVec3, balance: Balance) -> Option<DVec3> {
    Calibration { dmin, dmax }.validate().ok()?;
    let old_span = DVec3::from_array(std::array::from_fn(|c| log_ratio(dmin[c], dmax[c])));
    // Preserve the geometric mean of the old density spans without using it to set balance.
    let depth = (old_span.ln().element_sum() / 3.).exp();
    let span = balance.offsets().ok()?.exp() * depth;
    let inferred = dmin * (-span).exp();
    Calibration {
        dmin,
        dmax: inferred,
    }
    .validate()
    .ok()?;
    Some(inferred)
}
