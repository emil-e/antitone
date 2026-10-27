//! Repair evaluation from resolved working-positive samples.
#[cfg(all(test, feature = "project", feature = "ocio-reference"))]
#[path = "repair_source_normalization_tests.rs"]
mod normalization_tests;

use crate::{
    Error,
    dust_repair::{self, EvaluatedRepair, Execution, Repair},
    scan::{ImageSource, Scan},
};

/// Every repair reads the same unrepaired working positive, evaluated in f64 then stored as f32.
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

/// Resolve repairs influencing the captured output's declared processing support.
#[cfg(feature = "project")]
pub fn resolve_output(
    scan: &Scan,
    recipe: &crate::image_recipe::ImageRecipe,
    support: crate::processing_support::Support,
) -> Result<Vec<std::sync::Arc<EvaluatedRepair>>, Error> {
    resolve_output_with(scan, recipe, support, &mut Execution::default())
        .map(|result| result.expect("uncancelled output repair resolution"))
}

#[cfg(feature = "project")]
pub(crate) fn resolve_output_with(
    scan: &Scan,
    recipe: &crate::image_recipe::ImageRecipe,
    support: crate::processing_support::Support,
    execution: &mut Execution<'_>,
) -> Result<Option<Vec<std::sync::Arc<EvaluatedRepair>>>, Error> {
    use crate::geometry::FramePoint;
    let size = recipe.composition.size();
    let coverage = recipe
        .composition
        .coverage(FramePoint([0.; 2]), FramePoint(size.map(f64::from)))?;
    resolve_matching(scan, recipe, execution, |repair| {
        repair
            .support
            .selected_points()
            .any(|point| support.contributes(point, coverage, scan.area))
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
    let source = ImageSource::new(scan, recipe.neutral.processing_input()?)?;
    let mut evaluated = Vec::new();
    for repair in recipe.repairs().iter().filter(|repair| applicable(repair)) {
        let Some(result) = evaluate_with(&source, repair, execution)
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
