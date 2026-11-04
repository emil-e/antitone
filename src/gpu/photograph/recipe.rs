//! Recipe lowering happens at the operations that capture the corresponding graph arguments.

use super::*;
use crate::gpu::interpretation::InputTransform;
use crate::image_recipe::ImageRecipe;

#[cfg(all(test, feature = "render"))]
mod tests;

impl PhotographGraph {
    /// Request a photographic prefix or the inspection branch from the same corrected camera light.
    pub(crate) fn recipe_at(
        limits: &wgpu::Limits,
        source: Arc<ImageNode>,
        field: Option<Arc<GpuFlatField>>,
        recipe: &ImageRecipe,
        repairs: &crate::gpu::repairs::Snapshot,
        endpoint: Boundary,
    ) -> Result<Arc<ImageNode>, Error> {
        Self::require_recipe_source(&source, field.as_ref(), recipe)?;
        if endpoint == Boundary::Codes {
            return Ok(source);
        }
        if endpoint == Boundary::Camera {
            return Self::camera_light(source, field);
        }
        if let Boundary::Inspection(view) = endpoint {
            let matrix =
                glam::DMat3::from_cols_array_2d(&recipe.neutral.source_development.camera_to_ap1);
            return Self::inspection_original(source, field, matrix, view);
        }
        if matches!(
            endpoint,
            Boundary::Native | Boundary::Unrepaired | Boundary::Neutral
        ) {
            let program = InputTransform::from_recipe(&recipe.neutral)?;
            let mut assembly = Assembly::captured(Captured::new(
                CameraInput {
                    matrix: program.matrix,
                    field,
                    continuous: true,
                    mapping: Mapping::Originals,
                },
                Interpretation::Program(program, Default::default()),
            )?);
            if endpoint == Boundary::Neutral {
                repairs.require_area(source.domain().finite()?.source()?)?;
                assembly.repairs = repairs.clone();
            }
            return assembly.prefix(source, Boundary::Codes, endpoint);
        }
        let assembly = Self::recipe_assembly(limits, &source, field, recipe, repairs)?;
        if endpoint == Boundary::Processed {
            return Ok(assembly
                .photograph(source, &recipe.pixel_pipeline()?)?
                .processed);
        }
        assembly.prefix(source, Boundary::Codes, endpoint)
    }

    /// Capture the complete original-cell photograph, without acquiring or evaluating resources.
    pub(crate) fn from_recipe(
        limits: &wgpu::Limits,
        source: Arc<ImageNode>,
        field: Option<Arc<GpuFlatField>>,
        recipe: &ImageRecipe,
        repairs: &crate::gpu::repairs::Snapshot,
    ) -> Result<Self, Error> {
        Self::recipe_assembly(limits, &source, field, recipe, repairs)?
            .photograph(source, &recipe.pixel_pipeline()?)
    }

    #[cfg(feature = "editor")]
    pub(crate) fn filtered_recipe(
        limits: &wgpu::Limits,
        spec: &crate::gpu::view::request::SourceRequest,
        neutral: Arc<ImageNode>,
        recipe: &ImageRecipe,
    ) -> Result<(Arc<ImageNode>, crate::acquisition::Plan), Error> {
        if neutral.domain().meaning != Meaning::Neutral {
            return Err(Error::InvalidParameter(
                "a photograph requires its positive prefix",
            ));
        }
        let program = InputTransform::from_recipe(&recipe.neutral)?;
        let area = recipe.neutral.source_development.active_area.into();
        let grading = Grading::new(limits, recipe.roll, recipe.grade.clone(), area)?;
        grading.require_extent(spec.area)?;
        if Self::input_transform(&neutral).is_some_and(|captured| captured != program) {
            return Err(Error::Gpu("physical-view source is stale".into()));
        }
        admission::prepare(
            limits,
            program.input,
            recipe.roll,
            &recipe.grade,
            program.matrix,
            area,
            spec.repairs.bound(),
        )?;
        spec.repairs.require_area(area)?;
        let stages = recipe.pixel_pipeline()?;
        let plan = spec.processing_plan(&stages)?;
        let grading = grading.for_grid(neutral.domain().grid.coverage);
        let root = Self::filtered(&grading, neutral, &stages, spec.frame(), plan.output())?;
        Ok((root, plan))
    }

    pub(super) fn recipe_assembly(
        limits: &wgpu::Limits,
        source: &Arc<ImageNode>,
        field: Option<Arc<GpuFlatField>>,
        recipe: &ImageRecipe,
        repairs: &crate::gpu::repairs::Snapshot,
    ) -> Result<Assembly, Error> {
        Self::require_recipe_source(source, field.as_ref(), recipe)?;
        let program = InputTransform::from_recipe(&recipe.neutral)?;
        let area = source.domain().finite()?.source()?;
        Assembly::photographic(
            limits,
            CameraInput {
                matrix: program.matrix,
                field,
                continuous: true,
                mapping: Mapping::Originals,
            },
            program,
            Grading::new(limits, recipe.roll, recipe.grade.clone(), area)?,
            repairs.clone(),
        )
    }

    fn require_recipe_source(
        source: &Arc<ImageNode>,
        field: Option<&Arc<GpuFlatField>>,
        recipe: &ImageRecipe,
    ) -> Result<(), Error> {
        if source.domain().meaning != Meaning::CameraCodes
            || source.domain().grid != Grid::axis_aligned([0.; 2], [1.; 2])
            || source.domain().finite()?.source()?
                != recipe.neutral.source_development.active_area.into()
        {
            return Err(Error::InvalidParameter(
                "recipe requires its original camera cells",
            ));
        }
        if recipe.neutral.correction.as_ref() != field.map(|field| field.field()) {
            return Err(Error::Gpu(
                "recipe and GPU flat-field resources differ".into(),
            ));
        }
        Ok(())
    }
}
