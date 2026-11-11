//! Repair evaluation from resolved working-positive samples.
#[cfg(all(test, feature = "project", feature = "ocio-reference"))]
mod normalization_tests;

use crate::{
    Error,
    dust_repair::{self, EvaluatedRepair, Execution, Repair},
    scan::ImageSource,
};

/// Evaluate against this source's unrepaired working positive, storing f64 source values as f32.
pub fn evaluate(source: &ImageSource<'_>, repair: &Repair) -> Result<EvaluatedRepair, Error> {
    evaluate_with(source, repair, &mut Execution::default())
        .map(|result| result.expect("uncancelled repair evaluation"))
}

pub(crate) fn evaluate_with(
    source: &ImageSource<'_>,
    repair: &Repair,
    execution: &mut Execution<'_>,
) -> Result<Option<EvaluatedRepair>, Error> {
    if (execution.cancelled)() {
        return Ok(None);
    }
    dust_repair::evaluate_with(
        repair,
        source.area(),
        |point| {
            let value = source.sample(point)?.as_vec3().to_array();
            if value.iter().all(|channel| channel.is_finite()) {
                Ok(value)
            } else {
                Err(Error::NumericalOverflow)
            }
        },
        execution,
    )
}
