//! Photographic stage order, reusable prefixes and quality variants.
//!
//! Named endpoints place reconstruction and select repairs and film effects for each sampling path.
//! `film_effects` orders the shared post-grade stages.
//! Inspection branches before inversion; request routing and operation mathematics have separate owners.

use super::*;
#[cfg(feature = "editor")]
use crate::frame_raster::FrameRaster;
#[cfg(feature = "project")]
use crate::image_recipe::ImageRecipe;
use crate::photographic::stages::healing::operation as healing;
use crate::photographic::stages::interpretation::InputTransform;
use crate::photographic::stages::interpretation::operation::{Native, Working};
use crate::photographic::stages::{
    grain::GrainResponse, halation::HalationParameters, image_mtf::ImageMtfParameters,
};

#[cfg(all(test, feature = "editor"))]
mod sequence_tests;
#[cfg(all(test, feature = "project", feature = "render"))]
mod tests;

/// Reconstruction onto the presentation grid in a physical-view graph.
#[derive(Clone, Copy)]
pub(crate) struct PresentationMapping {
    domain: Domain,
    scale: [f64; 2],
}

impl PresentationMapping {
    #[cfg(feature = "editor")]
    pub(super) fn new(
        presentation: crate::frame_raster::FrameRaster,
        processing: crate::frame_raster::FrameRaster,
        coverage: u32,
    ) -> Self {
        Self {
            domain: view_domain(presentation, coverage),
            scale: std::array::from_fn(|i| {
                f64::from(processing.output[i]) / f64::from(presentation.output[i])
            }),
        }
    }

    pub(crate) fn compose(self, image: Value) -> Result<Value, Error> {
        if image.domain().same_grid(self.domain) {
            window(image, self.domain.finite()?)
        } else {
            reconstruct_cells(image, self.domain, [0.; 2], self.scale)
        }
    }
}

#[cfg(feature = "project")]
pub(crate) fn original_unrepaired(
    codes: Value,
    field: Option<Arc<GpuFlatField>>,
    recipe: &ImageRecipe,
) -> Result<Value, Error> {
    let matrix = original_basis(&codes, recipe)?;
    recipe_unrepaired(codes, field, recipe, matrix, Mapping::Originals, true)
}

#[cfg(feature = "project")]
fn original_basis(codes: &Value, recipe: &ImageRecipe) -> Result<glam::DMat3, Error> {
    if codes.domain().grid != Grid::axis_aligned([0.; 2], [1.; 2])
        || codes.domain().finite()?.source()?
            != recipe.neutral.source_development.active_area().into()
    {
        return Err(Error::InvalidParameter(
            "recipe requires its original camera cells",
        ));
    }
    recipe_basis(recipe)
}

#[cfg(feature = "project")]
fn recipe_basis(recipe: &ImageRecipe) -> Result<glam::DMat3, Error> {
    Ok(glam::DMat3::from_cols_array_2d(
        &recipe.neutral.source_development.resolved()?.camera_to_ap1,
    ))
}

#[cfg(all(test, feature = "project"))]
pub(crate) fn original_camera(
    codes: Value,
    field: Option<Arc<GpuFlatField>>,
    recipe: &ImageRecipe,
) -> Result<Value, Error> {
    let matrix = original_basis(&codes, recipe)?;
    recipe_camera(codes, field, recipe, matrix, Mapping::Originals, true)
}

#[cfg(feature = "project")]
fn recipe_camera(
    codes: Value,
    field: Option<Arc<GpuFlatField>>,
    recipe: &ImageRecipe,
    matrix: glam::DMat3,
    mapping: Mapping,
    continuous: bool,
) -> Result<Value, Error> {
    require_field(field.as_ref(), recipe)?;
    validate_camera_range(matrix, field.as_deref(), continuous)?;
    Ok(camera_image(codes, field, mapping))
}

#[cfg(feature = "project")]
fn recipe_unrepaired(
    codes: Value,
    field: Option<Arc<GpuFlatField>>,
    recipe: &ImageRecipe,
    matrix: glam::DMat3,
    mapping: Mapping,
    continuous: bool,
) -> Result<Value, Error> {
    let program = InputTransform::from_recipe(&recipe.neutral)?;
    validate_camera_range(program.matrix(), field.as_deref(), continuous)?;
    let camera = recipe_camera(codes, field, recipe, matrix, mapping, continuous)?;
    interpret_camera(camera, Interpretation::Program(program), program.matrix())
}

#[cfg(feature = "project")]
pub(super) fn require_field(
    field: Option<&Arc<GpuFlatField>>,
    recipe: &ImageRecipe,
) -> Result<(), Error> {
    if recipe.neutral.correction.as_ref() != field.map(|field| field.field()) {
        return Err(Error::Gpu(
            "recipe and GPU flat-field resources differ".into(),
        ));
    }
    Ok(())
}

#[cfg(feature = "project")]
pub(crate) fn original_neutral(
    codes: Value,
    field: Option<Arc<GpuFlatField>>,
    recipe: &ImageRecipe,
) -> Result<Value, Error> {
    let unrepaired = original_unrepaired(codes, field, recipe)?;
    healing::compose(unrepaired, &recipe.repairs)
}

/// Measurement endpoints use unrepaired original cells and stop before film effects.
#[cfg(feature = "editor")]
pub(crate) fn measured_roll(
    codes: Value,
    field: Option<Arc<GpuFlatField>>,
    recipe: &ImageRecipe,
) -> Result<Value, Error> {
    Grading::roll_image(original_unrepaired(codes, field, recipe)?, recipe.roll)
}

#[cfg(feature = "editor")]
pub(crate) fn measured_grade(
    limits: &wgpu::Limits,
    codes: Value,
    field: Option<Arc<GpuFlatField>>,
    recipe: &ImageRecipe,
) -> Result<Value, Error> {
    let roll = measured_roll(codes, field, recipe)?;
    recipe_grading(limits, recipe, Mapping::Originals)?.layers(roll)
}

#[cfg(feature = "project")]
pub(crate) fn processed_originals(
    limits: &wgpu::Limits,
    codes: Value,
    field: Option<Arc<GpuFlatField>>,
    recipe: &ImageRecipe,
) -> Result<Value, Error> {
    let neutral = original_neutral(codes, field, recipe)?;
    finish_on_input_grid(limits, neutral, recipe, Mapping::Originals)
}

#[cfg(feature = "project")]
fn finish_on_input_grid(
    limits: &wgpu::Limits,
    neutral: Value,
    recipe: &ImageRecipe,
    mapping: Mapping,
) -> Result<Value, Error> {
    let roll = Grading::roll_image(neutral, recipe.roll)?;
    let graded = recipe_grading(limits, recipe, mapping)?.layers(roll)?;
    film_on_input_grid(graded, recipe)
}

#[cfg(feature = "project")]
pub(super) fn film_on_input_grid(graded: Value, recipe: &ImageRecipe) -> Result<Value, Error> {
    let (grain, halation) = film_parameters(recipe)?;
    film_effects(graded, grain, halation, recipe.neutral.source.0, None)
}

#[cfg(all(feature = "project", feature = "ui", feature = "render"))]
pub(super) fn sampled_unrepaired(
    codes: Value,
    source: &ProcessingSource,
    field: Option<Arc<GpuFlatField>>,
    recipe: &ImageRecipe,
) -> Result<Value, Error> {
    let matrix = recipe_basis(recipe)?;
    if source.camera_matrix() != matrix {
        return Err(Error::Gpu(
            "recipe differs from resident camera basis".into(),
        ));
    }
    recipe_unrepaired(
        codes,
        field,
        recipe,
        matrix,
        Mapping::Resident(source.geometry()),
        source.continuous,
    )
}

#[cfg(all(feature = "project", feature = "ui", feature = "render"))]
pub(super) fn processed_resident(
    limits: &wgpu::Limits,
    source: &ProcessingSource,
    layout: [u32; 2],
    field: Option<Arc<GpuFlatField>>,
    recipe: &ImageRecipe,
) -> Result<Value, Error> {
    let raster = sampled_raster(source, recipe, layout)?;
    let codes = resident::codes(source, raster)?;
    // Original-coordinate repairs belong to the original-grid route.
    let unrepaired = sampled_unrepaired(codes, source, field, recipe)?;
    finish_on_input_grid(
        limits,
        unrepaired,
        recipe,
        Mapping::Resident(source.geometry()),
    )
}

#[cfg(feature = "project")]
pub(super) fn recipe_grading(
    limits: &wgpu::Limits,
    recipe: &ImageRecipe,
    mapping: Mapping,
) -> Result<Grading, Error> {
    let area = recipe.neutral.source_development.active_area().into();
    let mut grading = Grading::new(limits, recipe.roll, recipe.grade.clone(), area)?;
    if let Mapping::Resident(geometry) = mapping {
        grading.require_geometry(geometry)?;
        if let Some(geometry) = geometry.filter(|geometry| geometry.is_pair())
            && geometry.regions().any(|region| {
                region.x < area.x
                    || region.y < area.y
                    || region.x >= area.x + area.width
                    || region.y >= area.y + area.height
            })
        {
            return Err(Error::Gpu(
                "composed input and source geometry regions differ".into(),
            ));
        }
        grading = grading.with_camera_coordinates(geometry);
    }
    // Preserve admission of the combined input/Roll constants used by photographic controls.
    let program = InputTransform::from_recipe(&recipe.neutral)?;
    crate::gpu::parameters::ProcessingParameters::interpret(
        program.input(),
        recipe.roll,
        program.matrix(),
    )?;
    Ok(grading)
}

#[cfg(feature = "editor")]
pub(super) fn plan_sampling(
    recipe: &ImageRecipe,
    frame: FrameRaster,
    sampling: crate::acquisition::Sampling,
) -> Result<super::sampling::ViewSampling, Error> {
    InputTransform::from_recipe(&recipe.neutral)?.neutral_parameters()?;
    let area = recipe.neutral.source_development.active_area().into();
    let grading = Grading::new(
        &wgpu::Limits::default(),
        recipe.roll,
        recipe.grade.clone(),
        area,
    )?;
    let (grain, halation) = film_parameters(recipe)?;
    super::sampling::ViewSampling::new(
        frame,
        preferred_view_pitch(grain, halation),
        sampling,
        |input| {
            film_effects(
                grading.compose(input)?,
                grain,
                halation,
                recipe.neutral.source.0,
                None,
            )
        },
    )
}

#[cfg(feature = "editor")]
fn physical_view(
    limits: &wgpu::Limits,
    codes: Value,
    field: Option<Arc<GpuFlatField>>,
    recipe: &ImageRecipe,
    frame: crate::frame_raster::FrameRaster,
) -> Result<(Value, crate::acquisition::Plan), Error> {
    use super::sampling::ViewSampling;
    use crate::acquisition::Sampling;

    let matrix = original_basis(&codes, recipe)?;
    let grading = recipe_grading(limits, recipe, Mapping::Originals)?.for_grid(8);
    let (grain, halation) = film_parameters(recipe)?;
    let seed = recipe.neutral.source.0;
    let plan = ViewSampling::new(
        frame,
        preferred_view_pitch(grain, halation),
        Sampling::NeutralFootprints,
        |input| film_effects(grading.compose(input)?, grain, halation, seed, None),
    )?
    .plan;

    let unrepaired = recipe_unrepaired(codes, field, recipe, matrix, Mapping::Originals, true)?;
    let repaired = healing::compose(unrepaired, &recipe.repairs)?;
    let neutral = reconstruct_neutral(repaired, plan.output())?;
    let graded = grading.compose(neutral)?;
    let output = PresentationMapping::new(frame, plan.output(), 8);
    let processed = film_effects(graded, grain, halation, seed, Some(output))?;
    Ok((processed, plan))
}

#[cfg(feature = "editor")]
pub(crate) fn preview(
    limits: &wgpu::Limits,
    scan: &crate::scan::Scan,
    field: Option<Arc<GpuFlatField>>,
    recipe: &ImageRecipe,
    frame: crate::frame_raster::FrameRaster,
) -> Result<(Value, crate::acquisition::Plan), Error> {
    let program = InputTransform::from_recipe(&recipe.neutral)?;
    if !program.matches_native(scan)?
        || scan.flat_field() != recipe.neutral.correction.as_ref()
        || recipe.neutral.correction.as_ref() != field.as_ref().map(|field| field.field())
    {
        return Err(Error::Gpu(
            "physical-view source authority differs from originals".into(),
        ));
    }
    let source = crate::gpu::source::camera_source(scan)?;
    if frame.composition.area() != source.domain().finite()?.source()? {
        return Err(Error::InvalidParameter(
            "frame belongs to a different source grid",
        ));
    }
    physical_view(limits, source, field, recipe, frame)
}

#[cfg(all(feature = "project", feature = "ui", feature = "render"))]
fn sampled_raster(
    source: &ProcessingSource,
    recipe: &ImageRecipe,
    layout: [u32; 2],
) -> Result<crate::gpu::raster::Raster, Error> {
    if source.geometry().is_none()
        && (recipe
            .grain
            .parameters(recipe.composition.crop().size)?
            .is_some()
            || recipe
                .halation
                .parameters(recipe.composition.crop().size)?
                .is_some())
    {
        return Err(Error::Gpu(
            "photographic processing requires declared raster geometry".into(),
        ));
    }
    resident::raster(source, layout)
}

#[cfg(feature = "editor")]
pub(crate) fn reconstruct_neutral(neutral: Value, working: FrameRaster) -> Result<Value, Error> {
    // Repair before filtering. The filtered positive is reusable across grading edits.
    let mut grid = Grid::from_frame(working);
    grid.coverage = 8;
    Ok(
        crate::reconstruction::reconstruct_tiled(neutral, Domain { bounds: None, grid })?
            .for_reuse(),
    )
}

pub(super) fn native_image(camera: Value, interpretation: Interpretation) -> Result<Value, Error> {
    Ok(Value::from_operation(
        camera.domain(),
        Some(camera),
        Native::new(interpretation)?,
    ))
}

fn working_image(
    native: Value,
    interpretation: Interpretation,
    matrix: glam::DMat3,
) -> Result<Value, Error> {
    Ok(Value::from_operation(
        native.domain(),
        Some(native),
        Working::new(interpretation, matrix)?,
    ))
}

pub(super) fn interpret_camera(
    camera: Value,
    interpretation: Interpretation,
    matrix: glam::DMat3,
) -> Result<Value, Error> {
    working_image(
        native_image(camera, interpretation)?,
        interpretation,
        matrix,
    )
}

pub(super) fn repair_neutral(
    image: Value,
    mapping: Mapping,
    repairs: &[crate::dust_repair::Repair],
    #[cfg(test)] replacements: &[crate::photographic::stages::healing::fixtures::Replacement],
) -> Result<Value, Error> {
    let needed = !repairs.is_empty();
    #[cfg(test)]
    let needed = needed || !replacements.is_empty();
    if !needed {
        return Ok(image);
    }
    let domain = image.domain();
    // Resident original-cell arguments have local indices; masks name absolute source cells.
    let image = if domain.grid != Grid::axis_aligned([0.; 2], [1.; 2]) {
        let Mapping::Resident(Some(geometry)) = mapping else {
            return Err(Error::InvalidParameter(
                "healing requires original neutral cells",
            ));
        };
        let region = geometry.region();
        if geometry.is_pair()
            || geometry.size() != [region.width, region.height]
            || domain.grid.axes != [[1., 0.], [0., 1.]]
            || domain.grid.origin != [f64::from(region.x), f64::from(region.y)]
        {
            return Err(Error::InvalidParameter(
                "healing requires original neutral cells",
            ));
        }
        crate::reconstruction::reconstruct(
            image,
            Domain {
                bounds: Some(region.try_into()?),
                grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            },
        )?
    } else {
        image
    };
    let image = if repairs.is_empty() {
        image
    } else {
        healing::compose(image, repairs)?
    };
    #[cfg(test)]
    let image = if replacements.is_empty() {
        image
    } else {
        healing::compose_images(image, replacements.to_vec())?
    };
    if image.domain() == domain {
        Ok(image)
    } else {
        crate::reconstruction::reconstruct(image, domain)
    }
}

#[cfg(feature = "project")]
pub(super) fn film_parameters(
    recipe: &ImageRecipe,
) -> Result<(Option<GrainResponse>, Option<HalationParameters>), Error> {
    let crop = recipe.composition.crop().size;
    Ok((
        recipe.grain.parameters(crop)?,
        recipe.halation.parameters(crop)?,
    ))
}

#[cfg(any(test, feature = "editor"))]
pub(super) fn preferred_view_pitch(
    grain: Option<GrainResponse>,
    halation: Option<HalationParameters>,
) -> Option<f64> {
    grain
        .map(GrainResponse::preferred_view_pitch)
        .into_iter()
        .chain(halation.and_then(HalationParameters::preferred_view_pitch))
        .reduce(f64::min)
}

pub(super) fn film_effects(
    mut image: Value,
    grain: Option<GrainResponse>,
    halation: Option<HalationParameters>,
    seed: u64,
    output: Option<PresentationMapping>,
) -> Result<Value, Error> {
    if let Some(film) = grain {
        image = ImageMtfParameters {
            sigma: film.image_mtf_sigma,
        }
        .compose(image)?;
    }
    if let Some(halation) = halation {
        image = halation.compose(image)?;
    }
    if let Some(output) = output {
        image = output.compose(image)?;
    }
    if let Some(film) = grain.filter(|film| film.amplitude > 0.) {
        image = film.grain().compose(image, seed);
    }
    #[cfg(test)]
    let image = super::authoring::suffix(image)?;
    Ok(image)
}

#[cfg(feature = "editor")]
pub(crate) fn filtered_recipe(
    limits: &wgpu::Limits,
    spec: &crate::photographic::graph::view_source::SourceRequest,
    neutral: Value,
    recipe: &ImageRecipe,
) -> Result<(Value, crate::acquisition::Plan), Error> {
    let program = InputTransform::from_recipe(&recipe.neutral)?;
    let area = recipe.neutral.source_development.active_area().into();
    let grading = Grading::new(limits, recipe.roll, recipe.grade.clone(), area)?;
    grading.require_extent(spec.area)?;
    if input_transform(&neutral)?.is_some_and(|captured| captured != program) {
        return Err(Error::Gpu("physical-view source is stale".into()));
    }
    admission::prepare(
        limits,
        program.input(),
        recipe.roll,
        &recipe.grade,
        program.matrix(),
        area,
    )?;
    let coverage = neutral.domain().grid.coverage;
    let grading = grading.for_grid(coverage);
    let (grain, halation) = film_parameters(recipe)?;
    let seed = recipe.neutral.source.0;
    let plan =
        super::sampling::processing_plan(spec, preferred_view_pitch(grain, halation), |input| {
            film_effects(grading.compose(input)?, grain, halation, seed, None)
        })?;
    let output = PresentationMapping::new(spec.frame(), plan.output(), coverage);
    let graded = grading.compose(neutral)?;
    Ok((
        film_effects(graded, grain, halation, seed, Some(output))?,
        plan,
    ))
}

pub(super) fn inspection(
    source: Value,
    field: Option<Arc<GpuFlatField>>,
    mapping: Mapping,
    matrix: glam::DMat3,
    continuous: bool,
    view: crate::photographic::stages::interpretation::InspectionView,
) -> Result<Value, Error> {
    let interpretation = Interpretation::Packed(view.parameters()?);
    validate_camera_range(matrix, field.as_deref(), continuous)?;
    interpret_camera(camera_image(source, field, mapping), interpretation, matrix)
}

/// Build an original-grid working-positive calculation from explicit stage inputs.
pub fn original_image(
    limits: &wgpu::Limits,
    source: Value,
    field: Option<Arc<GpuFlatField>>,
    program: crate::photographic::stages::interpretation::InputTransform,
    grading: Grading,
    repairs: &[crate::dust_repair::Repair],
) -> Result<Value, Error> {
    require_original_grid(&source, &grading)?;
    admit_photograph(limits, program, &grading)?;
    validate_camera_range(program.matrix(), field.as_deref(), true)?;
    let camera = camera_image(source, field, Mapping::Originals);
    graded_source(camera, program, Mapping::Originals, &grading, repairs)
}
pub(super) fn require_original_grid(source: &Value, grading: &Grading) -> Result<(), Error> {
    if source.domain().grid != Grid::axis_aligned([0.; 2], [1.; 2]) {
        return Err(Error::InvalidParameter(
            "photographic conversion requires original camera cells",
        ));
    }
    grading.require_extent(source.domain().finite()?.source()?)
}

pub(super) fn admit_photograph(
    limits: &wgpu::Limits,
    program: InputTransform,
    grading: &Grading,
) -> Result<(), Error> {
    admission::prepare(
        limits,
        program.input(),
        grading.roll_settings(),
        grading.snapshot(),
        program.matrix(),
        grading.area(),
    )
    .map(|_| ())
}

pub(super) fn graded_source(
    camera: Value,
    program: InputTransform,
    mapping: Mapping,
    grading: &Grading,
    repairs: &[crate::dust_repair::Repair],
) -> Result<Value, Error> {
    let unrepaired = interpret_camera(camera, Interpretation::Program(program), program.matrix())?;
    let neutral = repair_neutral(
        unrepaired,
        mapping,
        repairs,
        #[cfg(test)]
        &[],
    )?;
    film_effects(grading.compose(neutral)?, None, None, 0, None)
}

pub(crate) fn inspection_original(
    source: Value,
    field: Option<Arc<GpuFlatField>>,
    matrix: glam::DMat3,
    view: crate::photographic::stages::interpretation::InspectionView,
) -> Result<Value, Error> {
    if source.domain().grid != Grid::axis_aligned([0.; 2], [1.; 2]) {
        return Err(Error::InvalidParameter(
            "inspection requires original camera cells",
        ));
    }
    inspection(source, field, Mapping::Originals, matrix, true, view)
}

#[cfg(feature = "editor")]
pub(crate) fn inspection_view(
    neutral: Value,
    parameters: ProcessingParameters,
    presentation: crate::frame_raster::FrameRaster,
) -> Result<Value, Error> {
    let output = view_domain(presentation, neutral.domain().grid.coverage);
    window(
        crate::photographic::stages::interpretation::inspect_filtered(
            neutral.for_reuse(),
            parameters,
        )?,
        output.finite()?,
    )
}
#[cfg(feature = "render")]
pub fn display_output(
    image: Value,
    luts: &crate::looks::LutChain,
    looks: &crate::photographic::stages::look::Looks,
    drt: &crate::photographic::stages::display_rendering::Target,
) -> Result<Value, Error> {
    drt.apply(looks.apply(image, luts)?)
}

#[cfg(feature = "render")]
pub(crate) fn prewarm_output(
    context: &Context,
    drt: &crate::photographic::stages::display_rendering::Targets,
) -> Result<(), Error> {
    use crate::rendering::OutputTarget;

    let input = borrowed_input(Domain {
        bounds: Some(CellRegion::new([0, 0], [1, 1])),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
    });
    let looked = crate::photographic::stages::look::prewarm_value(input.clone())?;
    context.prewarm_image(looked.clone())?;
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let display = drt.get(target);
        context.prewarm_texture(display.apply(input.clone())?)?;
        context.prewarm_texture(display.apply(looked.clone())?)?;
    }
    Ok(())
}
