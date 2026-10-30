//! Calibration requests original-grid camera light or a captured photographic neutral endpoint.

use super::*;
use crate::{
    Error,
    coverage::Coverage,
    editor::picking::measurement::query::{FORWARD_RESIDUAL_FLOOR, Samples},
    sampling::GammaNeutral,
    scan::Scan,
};

pub(super) fn reference(
    services: &super::super::source::Services,
    scan: &mut Scan,
    coverage: Coverage,
    cancelled: &dyn Fn() -> bool,
) -> Result<DVec3, Error> {
    let floor = f64::from(scan.effective_floor() as f32);
    let samples = Samples::new(services, scan, coverage, cancelled)?;
    let mean = samples.mean(samples.camera_light()?, cancelled)?;
    if mean.min_element() <= floor {
        return Err(Error::UnusableSample(
            "reference mean is at/below the scan precision floor",
        ));
    }

    Ok(mean)
}

pub(super) fn gamma(
    services: &super::super::source::Services,
    scan: &mut Scan,
    coverage: Coverage,
    input: InputSettings,
    cancelled: &dyn Fn() -> bool,
) -> Result<Balance, Error> {
    let calibration = input.calibration().ok_or(Error::InvalidParameter(
        "gamma fitting requires a negative input",
    ))?;
    let negative = input
        .with_calibration(scan.processing_calibration(calibration)?)
        .negative_input()
        .ok_or(Error::InvalidParameter(
            "gamma fitting requires a negative input",
        ))?;

    let matrix = scan.camera_to_ap1;
    let fit = GammaNeutral::new(negative, matrix)?;

    let samples = Samples::new(services, scan, coverage, cancelled)?;
    samples.require_precision(cancelled)?;

    let result = fit.fit(
        |candidate| {
            let readout = samples.camera_response(candidate)?;
            Ok(samples.mean(readout.image, cancelled)? * readout.decode_gain)
        },
        FORWARD_RESIDUAL_FLOOR,
    );

    if cancelled() {
        return Err(Error::Cancelled);
    }

    result
}
