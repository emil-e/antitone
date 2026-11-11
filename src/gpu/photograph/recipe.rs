//! Complete intent becomes a graph here. Source sampling and view quality determine where
//! reconstruction belongs; consumers choose endpoints, not photographic stage order.

use super::*;
use crate::gpu::interpretation::InputTransform;
use crate::image_recipe::ImageRecipe;
use crate::image_recipe::quality::sampling_stages;
use crate::pixel_pipeline::EffectDefinition;

#[cfg(all(test, feature = "render"))]
mod tests;

use crate::image_recipe::quality::Quality;

/// A captured source branch remains usable before inversion or grading is ready.
/// Asking for positive endpoints performs the corresponding numerical admission.
pub(crate) struct RecipeGraph<'a> {
    recipe: &'a ImageRecipe,
    pub source: CameraGraph,
    repairs: crate::gpu::repairs::Snapshot,
    quality: Quality,
}

impl<'a> RecipeGraph<'a> {
    pub fn originals(
        source: Arc<ImageNode>,
        field: Option<Arc<GpuFlatField>>,
        recipe: &'a ImageRecipe,
        repairs: &crate::gpu::repairs::Snapshot,
    ) -> Result<Self, Error> {
        PhotographGraph::require_recipe_source(&source, field.as_ref(), recipe)?;
        Ok(Self {
            recipe,
            source: CameraGraph::new(
                source,
                CameraInput {
                    matrix: glam::DMat3::from_cols_array_2d(
                        &recipe.neutral.source_development.resolved()?.camera_to_ap1,
                    ),
                    field,
                    continuous: true,
                    mapping: Mapping::Originals,
                },
            )?,
            repairs: repairs.clone(),
            quality: Quality::Originals,
        })
    }

    #[cfg(all(feature = "ui", feature = "render"))]
    pub fn sampled_camera(
        codes: Arc<ImageNode>,
        source: &ProcessingSource,
        field: Option<Arc<GpuFlatField>>,
        recipe: &'a ImageRecipe,
    ) -> Result<Self, Error> {
        let expected = glam::DMat3::from_cols_array_2d(
            &recipe.neutral.source_development.resolved()?.camera_to_ap1,
        );
        if source.camera_matrix() != expected {
            return Err(Error::Gpu(
                "recipe differs from resident camera basis".into(),
            ));
        }
        Self::require_field(field.as_ref(), recipe)?;
        Ok(Self {
            recipe,
            source: CameraGraph::new(
                codes,
                CameraInput {
                    matrix: expected,
                    field,
                    continuous: source.continuous,
                    mapping: Mapping::Resident(source.geometry()),
                },
            )?,
            repairs: Default::default(),
            quality: Quality::CameraProxy,
        })
    }

    fn require_field(field: Option<&Arc<GpuFlatField>>, recipe: &ImageRecipe) -> Result<(), Error> {
        if recipe.neutral.correction.as_ref() != field.map(|field| field.field()) {
            return Err(Error::Gpu(
                "recipe and GPU flat-field resources differ".into(),
            ));
        }
        Ok(())
    }

    fn interpretation(&self) -> Result<Assembly, Error> {
        let program = InputTransform::from_recipe(&self.recipe.neutral)?;
        let mut camera = self.source.input.clone();
        camera.matrix = program.matrix;
        Ok(Assembly::captured(Captured::new(
            camera,
            Interpretation::Program(program, Default::default()),
        )?))
    }

    pub(crate) fn interpreted(&self) -> Result<assembly::SourceImages, Error> {
        self.interpretation()?
            .interpret_camera(self.source.codes.clone(), self.source.camera.clone())
    }

    fn assembly(&self, limits: &wgpu::Limits, coverage: u32) -> Result<Assembly, Error> {
        let program = InputTransform::from_recipe(&self.recipe.neutral)?;
        let area = self.recipe.neutral.source_development.active_area().into();
        let mut camera = self.source.input.clone();
        camera.matrix = program.matrix;
        let mut grading = Grading::new(limits, self.recipe.roll, self.recipe.grade.clone(), area)?
            .for_grid(coverage);
        if let Mapping::Resident(geometry) = camera.mapping {
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
        Assembly::photographic(
            limits,
            camera,
            program,
            grading,
            if self.quality.includes_repairs() {
                self.repairs.clone()
            } else {
                Default::default()
            },
        )
    }

    /// Original samples: correction → interpretation → AP1 → repairs → Roll → layers
    /// → MTF → halation → grain. Already-sampled camera inputs use the same stages,
    /// but their approximation has already averaged camera samples and omits repairs.
    pub fn photograph(&self, limits: &wgpu::Limits) -> Result<PhotographGraph, Error> {
        let assembly = self.assembly(limits, 0)?;
        let source =
            assembly.interpret_camera(self.source.codes.clone(), self.source.camera.clone())?;
        let neutral = assembly.repair(source.unrepaired.clone())?;
        let finish = finish(&assembly, neutral, self.recipe, None)?;
        Ok(PhotographGraph {
            codes: source.codes,
            camera: source.camera,
            camera_positive: source.camera_positive,
            unrepaired: source.unrepaired,
            neutral: finish.neutral,
            roll: finish.roll,
            graded: finish.graded,
            processed: finish.processed,
        })
    }

    #[cfg(feature = "editor")]
    pub fn physical_view(
        &self,
        limits: &wgpu::Limits,
        frame: crate::frame_raster::FrameRaster,
        native_grid: Option<crate::frame_raster::FrameRaster>,
    ) -> Result<(PhotographGraph, crate::acquisition::Plan), Error> {
        let stages = sampling_stages(self.recipe)?;
        let layout = crate::pixel_pipeline::sampling::ViewSampling::new(
            frame,
            &stages,
            crate::acquisition::Sampling::NeutralFootprints,
        )?;
        let plan = if let Some(grid) = native_grid {
            if grid.composition != layout.presentation.composition
                || grid.min != layout.presentation.min
                || grid.max != layout.presentation.max
            {
                return Err(Error::InvalidParameter(
                    "view grids describe different footprints",
                ));
            }
            if (0..2).any(|axis| grid.output[axis] < layout.plan.output().output[axis]) {
                return Err(Error::InvalidParameter(
                    "acquired view grid is too coarse for processing",
                ));
            }
            crate::acquisition::Plan::new(crate::acquisition::Request {
                output: grid,
                stages: &stages,
                sampling: crate::acquisition::Sampling::NeutralFootprints,
            })?
        } else {
            layout.plan
        };
        let assembly = self.assembly(limits, 8)?;
        let source =
            assembly.interpret_camera(self.source.codes.clone(), self.source.camera.clone())?;
        let repaired = assembly.repair(source.unrepaired.clone())?;
        // Repair original working positives, then filter before the view's nonlinear grade.
        let neutral = reconstruct(
            repaired,
            Domain {
                bounds: None,
                grid: Grid {
                    coverage: 8,
                    ..Grid::from_frame(plan.output())
                },
                meaning: Meaning::Neutral,
            },
        )?;
        let output = Presentation::new(layout.presentation, plan.output(), 8);
        let finish = finish(&assembly, neutral, self.recipe, Some(output))?;
        Ok((
            PhotographGraph {
                codes: source.codes,
                camera: source.camera,
                camera_positive: source.camera_positive,
                unrepaired: source.unrepaired,
                neutral: finish.neutral,
                roll: finish.roll,
                graded: finish.graded,
                processed: finish.processed,
            },
            plan,
        ))
    }

    #[cfg(all(feature = "ui", feature = "render"))]
    pub fn sampled_raster(
        source: &ProcessingSource,
        recipe: &ImageRecipe,
        layout: [u32; 2],
    ) -> Result<crate::gpu::raster::Raster, Error> {
        PhotographGraph::resident_raster(source, &sampling_stages(recipe)?, layout)
    }
}

struct Finished {
    neutral: Arc<ImageNode>,
    roll: Arc<ImageNode>,
    graded: Arc<ImageNode>,
    processed: Arc<ImageNode>,
}

/// Reduction is between spatial stages and footprint-integrated grain for filtered views.
struct Presentation {
    domain: Domain,
    scale: [f64; 2],
}

impl Presentation {
    #[cfg(feature = "editor")]
    fn new(
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

    fn reconstruct(&self, image: Arc<ImageNode>) -> Result<Arc<ImageNode>, Error> {
        if image.domain().same_grid(self.domain) {
            window(image, self.domain.finite()?)
        } else {
            reconstruct_cells(image, self.domain, [0.; 2], self.scale)
        }
    }
}

fn finish(
    assembly: &Assembly,
    neutral: Arc<ImageNode>,
    recipe: &ImageRecipe,
    presentation: Option<Presentation>,
) -> Result<Finished, Error> {
    let film = recipe.grain.parameters(recipe.composition.crop().size)?;
    let halation = recipe.halation.parameters(recipe.composition.crop().size)?;
    let (roll, graded) = assembly.grade(neutral.clone())?;
    let mut image = graded.clone();
    if let Some(parameters) = film {
        image = crate::image_mtf::ImageMtfParameters {
            sigma: parameters.image_mtf_sigma,
        }
        .compose(image)?;
    }
    if let Some(parameters) = halation {
        image = parameters.compose(image)?;
    }
    if let Some(presentation) = presentation {
        image = presentation.reconstruct(image)?;
    }
    if let Some(parameters) = film.filter(|parameters| parameters.amplitude > 0.) {
        image = crate::pixel_pipeline::Grain {
            parameters: parameters.grain(),
            seed: recipe.neutral.source.0,
        }
        .compose(image)?;
    }
    Ok(Finished {
        neutral,
        roll,
        graded,
        processed: image,
    })
}

impl PhotographGraph {
    #[cfg(feature = "editor")]
    pub(crate) fn preview(
        limits: &wgpu::Limits,
        scan: &crate::scan::Scan,
        field: Option<Arc<GpuFlatField>>,
        recipe: &crate::image_recipe::ImageRecipe,
        repairs: &crate::gpu::repairs::Snapshot,
        frame: crate::frame_raster::FrameRaster,
        native_grid: Option<crate::frame_raster::FrameRaster>,
    ) -> Result<(Self, crate::acquisition::Plan), Error> {
        let program = crate::gpu::interpretation::InputTransform::from_recipe(&recipe.neutral)?;
        if !program.matches_native(scan)?
            || scan.flat_field() != recipe.neutral.correction.as_ref()
            || recipe.neutral.correction.as_ref() != field.as_ref().map(|field| field.field())
        {
            return Err(Error::Gpu(
                "physical-view source authority differs from originals".into(),
            ));
        }
        let source = crate::gpu::evaluation::camera_source(scan)?;
        if frame.composition.area() != source.domain().finite()?.source()? {
            return Err(Error::InvalidParameter(
                "frame belongs to a different source grid",
            ));
        }
        RecipeGraph::originals(source, field, recipe, repairs)?.physical_view(
            limits,
            frame,
            native_grid,
        )
    }

    pub(crate) fn from_recipe(
        limits: &wgpu::Limits,
        source: Arc<ImageNode>,
        field: Option<Arc<GpuFlatField>>,
        recipe: &ImageRecipe,
        repairs: &crate::gpu::repairs::Snapshot,
    ) -> Result<Self, Error> {
        RecipeGraph::originals(source, field, recipe, repairs)?.photograph(limits)
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
        let area = recipe.neutral.source_development.active_area().into();
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
        let plan = spec.processing_plan(&sampling_stages(recipe)?)?;
        let assembly = Assembly::filtered(grading.for_grid(neutral.domain().grid.coverage));
        let output = Presentation::new(spec.frame(), plan.output(), neutral.domain().grid.coverage);
        let finished = finish(&assembly, neutral, recipe, Some(output))?;
        Ok((finished.processed, plan))
    }

    fn require_recipe_source(
        source: &Arc<ImageNode>,
        field: Option<&Arc<GpuFlatField>>,
        recipe: &ImageRecipe,
    ) -> Result<(), Error> {
        if source.domain().meaning != Meaning::CameraCodes
            || source.domain().grid != Grid::axis_aligned([0.; 2], [1.; 2])
            || source.domain().finite()?.source()?
                != recipe.neutral.source_development.active_area().into()
        {
            return Err(Error::InvalidParameter(
                "recipe requires its original camera cells",
            ));
        }
        RecipeGraph::require_field(field, recipe)
    }
}

#[cfg(feature = "editor")]
impl crate::gpu::view::request::SourceRequest {
    pub fn for_recipe(
        frame: crate::frame_raster::FrameRaster,
        recipe: &ImageRecipe,
    ) -> Result<Self, Error> {
        let program = InputTransform::from_recipe(&recipe.neutral)?;
        program.neutral_parameters()?;
        let request = Self::for_frame(frame)?;
        #[cfg(test)]
        let request = request.with_program_fixture(program);
        request.with_pipeline(sampling_stages(recipe)?)
    }
}

/// The two nodes needed by view acquisition and downstream evaluation.
#[cfg(feature = "editor")]
pub(crate) struct ViewGraph {
    pub request: crate::gpu::view::request::SourceRequest,
    pub baseline: Arc<ImageNode>,
    pub processed: Arc<ImageNode>,
    pub plan: crate::acquisition::Plan,
}

#[cfg(feature = "editor")]
impl super::request::PhotographRequest {
    pub fn original(
        &self,
        device: &wgpu::Device,
        renderer: &crate::renderer::GpuRenderer,
        scan: &crate::scan::Scan,
        field: Option<Arc<crate::gpu::flat::GpuFlatField>>,
        repairs: &crate::gpu::repairs::Snapshot,
        target: crate::rendering::OutputTarget,
    ) -> Result<Arc<crate::gpu::evaluation::ImageNode>, Error> {
        if scan.flat_field() != field.as_ref().map(|field| field.field()) {
            return Err(Error::Gpu("CPU and GPU flat-field resources differ".into()));
        }
        let photograph = match self {
            Self::Recipe(recipe) | Self::BeforeLooks(recipe) => {
                recipe.neutral.require_scan(scan)?;
                if !crate::gpu::interpretation::InputTransform::from_recipe(&recipe.neutral)?
                    .matches_native(scan)?
                {
                    return Err(Error::Gpu(
                        "recipe interpretation differs from captured source".into(),
                    ));
                }
                super::RecipeGraph::originals(
                    crate::gpu::evaluation::camera_source(scan)?,
                    field,
                    recipe,
                    repairs,
                )?
                .photograph(&device.limits())?
                .processed
            }
            Self::Inspection(view) => super::PhotographGraph::inspection_original(
                crate::gpu::evaluation::camera_source(scan)?,
                field,
                scan.camera_to_ap1,
                *view,
            )?,
            #[cfg(test)]
            Self::Fixture(input) => {
                return crate::gpu::evaluation::original_chain(
                    device,
                    renderer,
                    scan,
                    field,
                    input,
                    crate::rendering::OutputReference::DisplayReferred(target),
                    &input.pixel_pipeline(),
                );
            }
        };
        crate::gpu::evaluation::Reference::capture(
            device,
            renderer,
            crate::rendering::OutputReference::DisplayReferred(target),
            self.luts(),
        )?
        .compose(photograph, renderer)
    }

    pub fn validate(
        &self,
        limits: &wgpu::Limits,
        repairs: &crate::gpu::repairs::Snapshot,
    ) -> Result<(), Error> {
        match self {
            Self::Recipe(recipe) | Self::BeforeLooks(recipe) => {
                let program = self.input_transform()?;
                super::admission::prepare(
                    limits,
                    program.input,
                    recipe.roll,
                    &recipe.grade,
                    program.matrix,
                    recipe.neutral.source_development.active_area().into(),
                    repairs.bound(),
                )
                .map(|_| ())
            }
            Self::Inspection(_) if repairs.0.is_empty() => Ok(()),
            Self::Inspection(_) => Err(Error::InvalidParameter(
                "inspection bypasses photographic repairs",
            )),
            #[cfg(test)]
            Self::Fixture(input) => input.validate_repairs(limits, repairs.bound()),
        }
    }

    pub fn source_request(
        &self,
        frame: crate::frame_raster::FrameRaster,
        repairs: crate::gpu::repairs::Snapshot,
    ) -> Result<crate::gpu::view::request::SourceRequest, Error> {
        use crate::gpu::view::request::SourceRequest;
        let request = match self {
            Self::Recipe(recipe) | Self::BeforeLooks(recipe) => {
                SourceRequest::for_recipe(frame, recipe)?
            }
            Self::Inspection(_) => SourceRequest::for_frame(frame)?,
            #[cfg(test)]
            Self::Fixture(input) if input.is_positive() => {
                SourceRequest::for_processing(frame, input)?
            }
            #[cfg(test)]
            Self::Fixture(_) => SourceRequest::for_frame(frame)?,
        };
        Ok(request.with_repairs(repairs))
    }

    pub fn input_transform(&self) -> Result<crate::gpu::interpretation::InputTransform, Error> {
        match self {
            Self::Recipe(recipe) | Self::BeforeLooks(recipe) => {
                crate::gpu::interpretation::InputTransform::from_recipe(&recipe.neutral)
            }
            Self::Inspection(_) => Err(Error::InvalidParameter(
                "a positive requires input interpretation",
            )),
            #[cfg(test)]
            Self::Fixture(input) => {
                input
                    .source_transform()
                    .copied()
                    .ok_or(Error::InvalidParameter(
                        "fixture has no input interpretation",
                    ))
            }
        }
    }

    pub fn neutral_input(
        &self,
        field: Option<Arc<crate::gpu::flat::GpuFlatField>>,
        repairs: crate::gpu::repairs::Snapshot,
        matrix: glam::DMat3,
        quality: Quality,
    ) -> Result<super::NeutralInput, Error> {
        use crate::gpu::interpretation::SourceInterpretation;
        let repairs = if self.recipe().is_some() && !quality.includes_repairs() {
            Default::default()
        } else {
            repairs
        };
        let interpretation = match self {
            Self::Recipe(recipe) | Self::BeforeLooks(recipe) => {
                if recipe.neutral.correction.as_ref() != field.as_ref().map(|field| field.field()) {
                    return Err(Error::Gpu(
                        "recipe and GPU flat-field resources differ".into(),
                    ));
                }
                SourceInterpretation::Photograph(self.input_transform()?)
            }
            Self::Inspection(_) => {
                if !repairs.0.is_empty() {
                    return Err(Error::InvalidParameter(
                        "inspection bypasses photographic repairs",
                    ));
                }
                SourceInterpretation::Inspection(matrix)
            }
            #[cfg(test)]
            Self::Fixture(input) => input
                .source_transform()
                .map_or(SourceInterpretation::Inspection(matrix), |program| {
                    SourceInterpretation::Photograph(*program)
                }),
        };
        Ok(super::NeutralInput {
            interpretation,
            field,
            repairs,
        })
    }

    pub fn sampled_graph(
        &self,
        limits: &wgpu::Limits,
        source: &ProcessingSource,
        field: Option<Arc<GpuFlatField>>,
        spec: crate::gpu::view::request::SourceRequest,
    ) -> Result<ViewGraph, Error> {
        let (request, baseline) = self.sampled_prefix(source, field, spec)?;
        let (processed, plan) = self.filtered(limits, &request, baseline.clone())?;
        Ok(ViewGraph {
            request,
            baseline,
            processed,
            plan,
        })
    }

    pub fn retained_graph(
        &self,
        limits: &wgpu::Limits,
        baseline: &Arc<ImageNode>,
        field: Option<Arc<GpuFlatField>>,
        request: &crate::gpu::view::request::SourceRequest,
    ) -> Result<ViewGraph, Error> {
        let baseline = self.retained_prefix(baseline, field, request.repairs.clone())?;
        let (processed, plan) = self.filtered(limits, request, baseline.clone())?;
        Ok(ViewGraph {
            request: request.clone(),
            baseline,
            processed,
            plan,
        })
    }

    pub fn sampled_prefix(
        &self,
        source: &ProcessingSource,
        field: Option<Arc<GpuFlatField>>,
        spec: crate::gpu::view::request::SourceRequest,
    ) -> Result<(crate::gpu::view::request::SourceRequest, Arc<ImageNode>), Error> {
        let geometry = source
            .geometry()
            .ok_or_else(|| Error::Gpu("preview source has no geometry".into()))?;
        let input = self.neutral_input(
            field.clone(),
            spec.repairs.clone(),
            source.camera_matrix(),
            Quality::CameraProxy,
        )?;
        if !input.repairs.0.is_empty() {
            return Err(Error::Gpu(
                "repaired previews require original-coordinate acquisition".into(),
            ));
        }
        let spec = spec.with_repairs(input.repairs.clone());
        let raster = crate::gpu::raster::Raster::region(geometry.region(), geometry.size());
        let root = if let Some(recipe) = self.recipe() {
            RecipeGraph::sampled_camera(
                PhotographGraph::resident_codes(source, raster)?,
                source,
                field,
                recipe,
            )?
            .interpreted()?
            .unrepaired
        } else {
            input.resident(source, raster)?
        };
        let output = match spec.plan.sampling() {
            crate::acquisition::Sampling::CameraCells { .. } => Domain {
                grid: Grid {
                    coverage: 8,
                    ..root.domain().grid
                },
                ..root.domain()
            },
            _ => Domain {
                bounds: None,
                grid: Grid {
                    coverage: 8,
                    ..Grid::from_frame(spec.plan.output())
                },
                meaning: root.domain().meaning,
            },
        };
        Ok((spec, reconstruct(root, output)?))
    }

    pub fn retained_prefix(
        &self,
        source: &Arc<ImageNode>,
        field: Option<Arc<GpuFlatField>>,
        repairs: crate::gpu::repairs::Snapshot,
    ) -> Result<Arc<ImageNode>, Error> {
        let program = self.input_transform()?;
        self.neutral_input(field, repairs, program.matrix, Quality::CameraProxy)?
            .recompose_resident(source)
    }

    pub fn require_proxy(&self, proxy: &crate::scan::CameraProxy) -> Result<(), Error> {
        if !self.input_transform()?.matches_proxy(proxy)? {
            return Err(Error::Gpu(
                "camera proxy does not match thumbnail source raster".into(),
            ));
        }
        Ok(())
    }

    pub fn filtered(
        &self,
        limits: &wgpu::Limits,
        spec: &crate::gpu::view::request::SourceRequest,
        neutral: Arc<crate::gpu::evaluation::ImageNode>,
    ) -> Result<
        (
            Arc<crate::gpu::evaluation::ImageNode>,
            crate::acquisition::Plan,
        ),
        Error,
    > {
        match self {
            Self::Recipe(recipe) | Self::BeforeLooks(recipe) => {
                super::PhotographGraph::filtered_recipe(limits, spec, neutral, recipe)
            }
            Self::Inspection(view) => {
                if neutral.domain().meaning != crate::gpu::evaluation::Meaning::Inspection
                    || !spec.repairs.0.is_empty()
                {
                    return Err(Error::InvalidParameter(
                        "inspection requires its source-view prefix",
                    ));
                }
                Ok((
                    super::PhotographGraph::inspection_view(
                        neutral,
                        view.parameters()?,
                        spec.frame(),
                    )?,
                    spec.plan.as_ref().clone(),
                ))
            }
            #[cfg(test)]
            Self::Fixture(input) => crate::gpu::view::processing::ProcessedView::definition(
                limits, spec, neutral, input,
            ),
        }
    }

    pub fn source_graphs(
        &self,
        scan: &crate::scan::Scan,
        field: Option<Arc<crate::gpu::flat::GpuFlatField>>,
        specs: &[crate::gpu::view::request::SourceRequest],
    ) -> Result<Vec<Arc<crate::gpu::evaluation::ImageNode>>, Error> {
        let source = crate::gpu::evaluation::camera_source(scan)?;
        specs
            .iter()
            .map(|spec| {
                let input = self.neutral_input(
                    field.clone(),
                    spec.repairs.clone(),
                    scan.camera_to_ap1,
                    Quality::PhysicalView,
                )?;
                input
                    .original_views(source.clone(), [spec.plan.output()])?
                    .pop()
                    .ok_or(Error::InvalidParameter(
                        "missing requested photographic prefix",
                    ))
            })
            .collect()
    }

    #[cfg(test)]
    pub fn processing(
        &self,
        limits: &wgpu::Limits,
        repairs: &crate::gpu::repairs::Snapshot,
    ) -> Result<AuthoredPhotograph, Error> {
        let mut input = match self {
            Self::Recipe(recipe) | Self::BeforeLooks(recipe) => {
                AuthoredPhotograph::from_recipe(limits, recipe)?
            }
            Self::Inspection(view) => AuthoredPhotograph::from(view.parameters()?),
            #[cfg(test)]
            Self::Fixture(input) => input.clone(),
        };
        input.repairs = repairs.clone();
        Ok(input)
    }
}
