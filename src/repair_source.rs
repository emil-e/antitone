//! Source-aware neutral-AP1 sampling shared by interactive and offscreen repair consumers.
#[cfg(all(test, feature = "project", feature = "ocio-reference"))]
#[path = "repair_source_normalization_tests.rs"]
mod normalization_tests;

use crate::{
    Error,
    dust_repair::{self, EvaluatedRepair, Execution, Repair},
    processing::{InputPipeline, InputSettings},
    scan::Scan,
};

/// The input is refined for this scan's correction context before interpreting.
/// Every repair reads immutable unrepaired camera samples, evaluated in f64 then stored as f32.
pub fn evaluate(
    scan: &Scan,
    input: &InputSettings,
    repair: &Repair,
) -> Result<EvaluatedRepair, Error> {
    evaluate_with(scan, input, repair, &mut Execution::default())
        .map(|result| result.expect("uncancelled repair evaluation"))
}

pub(crate) fn evaluate_with(
    scan: &Scan,
    input: &InputSettings,
    repair: &Repair,
    execution: &mut Execution<'_>,
) -> Result<Option<EvaluatedRepair>, Error> {
    if (execution.cancelled)() {
        return Ok(None);
    }
    let input = match input.calibration() {
        Some(base) => input.with_calibration(scan.processing_calibration(base)?),
        None => *input,
    };
    let pipeline = InputPipeline::ungraded(input, scan.positive_matrix(&input)?)?;
    dust_repair::evaluate_with(
        repair,
        scan.area,
        |[x, y]| {
            let rgb = pipeline.apply(scan.pixel(x, y)?)?.rgb;
            let value = rgb.as_vec3().to_array();
            if value.iter().all(|channel| channel.is_finite()) {
                Ok(value)
            } else {
                Err(Error::NumericalOverflow)
            }
        },
        execution,
    )
}

/// Resolve all enabled source repairs synchronously on the calling worker, without viewer state.
#[cfg(feature = "project")]
pub fn resolve(
    scan: &Scan,
    recipe: &crate::image_recipe::ImageRecipe,
) -> Result<Vec<std::sync::Arc<EvaluatedRepair>>, Error> {
    resolve_with(scan, recipe, &mut Execution::default())
        .map(|result| result.expect("uncancelled recipe resolution"))
}

#[cfg(feature = "project")]
pub(crate) fn resolve_with(
    scan: &Scan,
    recipe: &crate::image_recipe::ImageRecipe,
    execution: &mut Execution<'_>,
) -> Result<Option<Vec<std::sync::Arc<EvaluatedRepair>>>, Error> {
    resolve_matching(scan, recipe, execution, |_| true)
}

/// Resolve supports influencing the captured output, including the film image-MTF neighbourhood.
#[cfg(feature = "project")]
pub fn resolve_output(
    scan: &Scan,
    recipe: &crate::image_recipe::ImageRecipe,
) -> Result<Vec<std::sync::Arc<EvaluatedRepair>>, Error> {
    resolve_output_with(scan, recipe, &mut Execution::default())
        .map(|result| result.expect("uncancelled output repair resolution"))
}

#[cfg(feature = "project")]
pub(crate) fn resolve_output_with(
    scan: &Scan,
    recipe: &crate::image_recipe::ImageRecipe,
    execution: &mut Execution<'_>,
) -> Result<Option<Vec<std::sync::Arc<EvaluatedRepair>>>, Error> {
    use crate::geometry::FramePoint;
    let size = recipe.composition.size();
    let coverage = recipe
        .composition
        .coverage(FramePoint([0.; 2]), FramePoint(size.map(f64::from)))?;
    let radius = recipe
        .film_emulation
        .parameters(scan.area.height)?
        .map_or(0, |parameters| parameters.support_radius());
    resolve_matching(scan, recipe, execution, |repair| {
        repair.support.selected_points().any(|point| {
            if radius == 0 {
                return coverage.weight(point) > 0.;
            }
            let x = point[0].saturating_sub(radius).max(scan.area.x);
            let y = point[1].saturating_sub(radius).max(scan.area.y);
            let right = point[0]
                .saturating_add(radius)
                .saturating_add(1)
                .min(scan.area.x + scan.area.width);
            let bottom = point[1]
                .saturating_add(radius)
                .saturating_add(1)
                .min(scan.area.y + scan.area.height);
            coverage.intersection_area(crate::geometry::SourceRegion {
                x,
                y,
                width: right - x,
                height: bottom - y,
            }) > 0.
        })
    })
}

#[cfg(feature = "project")]
fn resolve_matching(
    scan: &Scan,
    recipe: &crate::image_recipe::ImageRecipe,
    execution: &mut Execution<'_>,
    applicable: impl Fn(&Repair) -> bool,
) -> Result<Option<Vec<std::sync::Arc<EvaluatedRepair>>>, Error> {
    recipe.neutral.require_scan(scan)?;
    let mut evaluated = Vec::new();
    for repair in recipe.repairs().iter().filter(|repair| applicable(repair)) {
        let Some(result) =
            evaluate_with(scan, &recipe.neutral.processing_input()?, repair, execution)
                .map_err(|error| Error::Raw(format!("repair {}: {error}", repair.id.0)))?
        else {
            return Ok(None);
        };
        evaluated.push(std::sync::Arc::new(result));
    }
    Ok((!(execution.cancelled)()).then_some(evaluated))
}

#[cfg(feature = "project")]
impl crate::image_recipe::NeutralRecipe {
    /// The consumer owns the immutable source lease and freshness fence, not the selected viewer.
    pub fn require_scan(&self, scan: &Scan) -> Result<(), Error> {
        let (width, height) = scan.dimensions()?;
        let area = scan.area;
        let actual = crate::project::SourceDevelopment {
            decode: scan
                .source()
                .metadata()
                .development(scan.source().fixed_wb()),
            lens_correction: scan.source().metadata().lens_identity()?,
            dng_development: scan.source().metadata().dng_identity(),
            dimensions: [width, height],
            active_area: [area.x, area.y, area.width, area.height],
            camera_model: scan.camera_model.clone(),
            camera_to_ap1: scan.camera_to_ap1.to_cols_array_2d(),
            input_floor: scan.source().input_floor(),
        };
        if self.source_development != actual || self.correction.as_ref() != scan.flat_field() {
            return Err(Error::Raw(
                "repair recipe differs from developed source".into(),
            ));
        }
        Ok(())
    }
}
