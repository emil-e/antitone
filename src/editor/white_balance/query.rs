//! Original-grid WB inputs evaluated through the shared photographic components.

use super::*;
use crate::{
    editor::measurement::query::{FORWARD_RESIDUAL_FLOOR, Samples},
    gpu::grading::Grading,
};
use glam::DVec3;

#[cfg(test)]
mod tests;

pub(super) fn roll(
    services: &super::super::source::Services,
    scan: &mut Scan,
    coverage: Coverage,
    input: crate::image_recipe::ImageRecipe,
    cancelled: &dyn Fn() -> bool,
) -> Result<Balance, Error> {
    if cancelled() {
        return Err(Error::Cancelled);
    }
    let (mut scan, input) = crate::photo_preparation::native_image(scan, &input)?;
    let samples = Samples::new(services, &mut scan, coverage, cancelled)?;
    samples.require_precision(cancelled)?;

    let photograph = samples.photograph(&input)?;

    crate::color::neutral_wb(samples.mean(photograph.roll, cancelled)?)
}

pub(super) fn layer(
    services: &super::super::source::Services,
    scan: &mut Scan,
    coverage: Coverage,
    input: LayerInput,
    cancelled: &dyn Fn() -> bool,
) -> Result<Balance, Error> {
    if cancelled() {
        return Err(Error::Cancelled);
    }
    let (mut scan, recipe) = crate::photo_preparation::native_image(scan, &input.recipe)?;
    let samples = Samples::new(services, &mut scan, coverage, cancelled)?;
    samples.require_precision(cancelled)?;

    let layer = recipe.grade.layers()[input.index];
    let weights = Grading::weight_image(samples.source.clone(), layer)?;
    let mut covered = false;
    samples.read(weights, cancelled, |_, pixel, _| {
        if !pixel[0].is_finite() || !(0. ..=1.).contains(&pixel[0]) {
            return Err(Error::UnusableSample("WB layer has invalid weights"));
        }
        covered |= pixel[0] > 0.;
        Ok(())
    })?;
    if !covered {
        return Err(Error::UnusableSample(
            "WB patch is outside the enabled layer's coverage",
        ));
    }

    let cones = crate::color::ap1_to_cones();
    let white = cones * DVec3::ONE;
    let result = crate::sampling::fit_white_balance_model(
        |coordinates| {
            let mut selected = layer;
            selected.photo.wb = Balance {
                warmth: coordinates.x,
                tint: coordinates.y,
            };
            selected.photo.contrast = 1.;
            selected.photo.saturation = 1.;

            let mut layers = recipe.grade.layers()[..input.index].to_vec();
            layers.push(selected);
            let mut candidate = recipe.clone();
            candidate.grade = AdjustmentLayerSnapshot::new(layers)?;
            let photograph = samples.photograph(&candidate)?;

            let response = cones * samples.mean(photograph.graded, cancelled)?;
            if !response.is_finite() || response.min_element() <= 0. {
                return Err(Error::UnusableSample(
                    "WB patch has invalid Bradford cone responses",
                ));
            }

            Ok(crate::color::opponent_coordinates((white / response).ln()))
        },
        Balance::default(),
        // Packed parameters and evaluated pixels are f32. Below this chromatic residual the
        // forward model can plateau even while the f64 solver continues proposing steps.
        FORWARD_RESIDUAL_FLOOR,
    );

    if cancelled() {
        return Err(Error::Cancelled);
    }

    result
}
