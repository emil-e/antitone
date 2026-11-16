//! Complete intent becomes a graph here. Source sampling and view quality determine where
//! reconstruction belongs; consumers choose endpoints, not photographic stage order.

#[cfg(feature = "editor")]
use super::assembly::EffectOutput;
use super::*;
use crate::gpu::interpretation::InputTransform;
use crate::image_recipe::ImageRecipe;
use crate::image_recipe::quality::sampling_stages;

#[cfg(all(test, feature = "render"))]
mod tests;

use crate::image_recipe::quality::Quality;

/// A captured source branch remains usable before inversion or grading is ready.
/// Asking for positive endpoints performs the corresponding numerical admission.
pub(crate) struct RecipeGraph<'a> {
    recipe: &'a ImageRecipe,
    source: CameraGraph,
    quality: Quality,
}

impl<'a> RecipeGraph<'a> {
    pub fn originals(
        source: Value,
        field: Option<Arc<GpuFlatField>>,
        recipe: &'a ImageRecipe,
    ) -> Result<Self, Error> {
        PhotographGraph::require_recipe_source(&source, recipe)?;
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
            quality: Quality::Originals,
        })
    }

    #[cfg(all(feature = "ui", feature = "render"))]
    pub fn sampled_camera(
        codes: Value,
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

    /// Original-grid measurements omit repairs and display emulation.
    #[cfg(feature = "editor")]
    pub(crate) fn measurement(mut self) -> Self {
        self.quality = Quality::Measurement;
        self
    }

    pub(crate) fn codes(&self) -> Value {
        self.source.codes.clone()
    }

    pub(crate) fn camera(&self) -> Result<Value, Error> {
        Self::require_field(self.source.input.field.as_ref(), self.recipe)?;
        self.source.camera()
    }

    pub(crate) fn camera_positive(&self) -> Result<Value, Error> {
        self.interpretation()?.interpret(self.camera()?)
    }

    pub(crate) fn unrepaired(&self) -> Result<Value, Error> {
        self.interpretation()?.working(self.camera_positive()?)
    }

    pub(crate) fn neutral(&self) -> Result<Value, Error> {
        self.repair(self.unrepaired()?)
    }

    pub(crate) fn roll(&self) -> Result<Value, Error> {
        Grading::roll_image(self.neutral()?, self.recipe.roll)
    }

    pub(crate) fn graded(&self, limits: &wgpu::Limits) -> Result<Value, Error> {
        let assembly = self.assembly(limits, 0)?;
        assembly
            .grading
            .as_ref()
            .expect("photographic grading")
            .layers(self.roll()?)
    }

    pub(crate) fn processed(&self, limits: &wgpu::Limits) -> Result<Value, Error> {
        self.stages()?.compose(self.graded(limits)?)
    }

    #[cfg(feature = "render")]
    pub(crate) fn output(
        &self,
        renderer: &crate::renderer::GpuRenderer,
        reference: crate::rendering::OutputReference,
    ) -> Result<Value, Error> {
        compose_output(
            renderer,
            self.processed(&renderer.evaluation().device().limits())?,
            reference,
            &self.recipe.luts,
        )
    }

    fn stages(&self) -> Result<crate::pixel_pipeline::PixelPipeline, Error> {
        #[cfg(feature = "editor")]
        if self.quality == Quality::Measurement {
            return Ok(Default::default());
        }
        sampling_stages(self.recipe)
    }

    fn interpretation(&self) -> Result<Assembly, Error> {
        let program = InputTransform::from_recipe(&self.recipe.neutral)?;
        let mut camera = self.source.input.clone();
        camera.matrix = program.matrix;
        Ok(Assembly::captured(Captured::new(
            camera,
            Interpretation::Program(program),
        )?))
    }

    pub(crate) fn interpreted(&self) -> Result<assembly::SourceImages, Error> {
        self.interpretation()?
            .interpret_camera(self.codes(), self.camera()?)
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
        Assembly::photographic(limits, camera, program, grading, Default::default())
    }

    fn repair(&self, image: Value) -> Result<Value, Error> {
        if self.quality.includes_repairs() {
            crate::gpu::repairs::definition::compose(image, &self.recipe.repairs)
        } else {
            Ok(image)
        }
    }

    /// Original samples: correction → interpretation → AP1 → repairs → Roll → layers
    /// → MTF → halation → grain. Already-sampled camera inputs use the same stages,
    /// but their approximation has already averaged camera samples and omits repairs.
    #[cfg(test)]
    pub fn photograph(&self, limits: &wgpu::Limits) -> Result<PhotographGraph, Error> {
        let assembly = self.assembly(limits, 0)?;
        let source = assembly.interpret_camera(self.codes(), self.camera()?)?;
        let neutral = self.repair(source.unrepaired.clone())?;
        Ok(assembly
            .finish(neutral, &self.stages()?, None)?
            .with_source(source))
    }

    #[cfg(feature = "editor")]
    pub fn physical_view(
        &self,
        limits: &wgpu::Limits,
        frame: crate::frame_raster::FrameRaster,
        native_grid: Option<crate::frame_raster::FrameRaster>,
    ) -> Result<(PhotographGraph, crate::acquisition::Plan), Error> {
        let stages = self.stages()?;
        let layout = crate::pixel_pipeline::sampling::ViewSampling::new(
            frame,
            &stages,
            crate::acquisition::Sampling::NeutralFootprints,
        )?
        .with_native_grid(native_grid)?;
        let plan = layout.plan;
        let assembly = self.assembly(limits, 8)?;
        let source = assembly.interpret_camera(self.codes(), self.camera()?)?;
        let repaired = self.repair(source.unrepaired.clone())?;
        // Repair original working positives, then filter before the view's nonlinear grade.
        let neutral = crate::gpu::evaluation::reconstruct_tiled(
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
        let output = EffectOutput::new(layout.presentation, plan.output(), 8);
        Ok((
            assembly
                .finish(neutral, &stages, Some(output))?
                .with_source(source),
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

impl ImageRecipe {
    pub(crate) fn validate(
        &self,
        limits: &wgpu::Limits,
    ) -> Result<crate::gpu::parameters::ProcessingParameters, Error> {
        let program = InputTransform::from_recipe(&self.neutral)?;
        admission::prepare(
            limits,
            program.input,
            self.roll,
            &self.grade,
            program.matrix,
            self.neutral.source_development.active_area().into(),
        )
    }
}

impl PhotographGraph {
    #[cfg(feature = "editor")]
    pub(crate) fn preview(
        limits: &wgpu::Limits,
        scan: &crate::scan::Scan,
        field: Option<Arc<GpuFlatField>>,
        recipe: &crate::image_recipe::ImageRecipe,
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
        RecipeGraph::originals(source, field, recipe)?.physical_view(limits, frame, native_grid)
    }

    #[cfg(test)]
    pub(crate) fn from_recipe(
        limits: &wgpu::Limits,
        source: Value,
        field: Option<Arc<GpuFlatField>>,
        recipe: &ImageRecipe,
    ) -> Result<Self, Error> {
        RecipeGraph::originals(source, field, recipe)?.photograph(limits)
    }

    #[cfg(feature = "editor")]
    pub(crate) fn filtered_recipe(
        limits: &wgpu::Limits,
        spec: &crate::gpu::view::request::SourceRequest,
        neutral: Value,
        recipe: &ImageRecipe,
    ) -> Result<(Value, crate::acquisition::Plan), Error> {
        if neutral.domain().meaning != Meaning::Neutral {
            return Err(Error::InvalidParameter(
                "a photograph requires its positive prefix",
            ));
        }
        let program = InputTransform::from_recipe(&recipe.neutral)?;
        let area = recipe.neutral.source_development.active_area().into();
        let grading = Grading::new(limits, recipe.roll, recipe.grade.clone(), area)?;
        grading.require_extent(spec.area)?;
        if Self::input_transform(&neutral)?.is_some_and(|captured| captured != program) {
            return Err(Error::Gpu("physical-view source is stale".into()));
        }
        admission::prepare(
            limits,
            program.input,
            recipe.roll,
            &recipe.grade,
            program.matrix,
            area,
        )?;
        let stages = sampling_stages(recipe)?;
        let plan = spec.processing_plan(&stages)?;
        let assembly = Assembly::filtered(grading.for_grid(neutral.domain().grid.coverage));
        let output = EffectOutput::new(spec.frame(), plan.output(), neutral.domain().grid.coverage);
        let finished = assembly.finish(neutral, &stages, Some(output))?;
        Ok((finished.processed, plan))
    }

    fn require_recipe_source(source: &Value, recipe: &ImageRecipe) -> Result<(), Error> {
        if source.domain().meaning != Meaning::CameraCodes
            || source.domain().grid != Grid::axis_aligned([0.; 2], [1.; 2])
            || source.domain().finite()?.source()?
                != recipe.neutral.source_development.active_area().into()
        {
            return Err(Error::InvalidParameter(
                "recipe requires its original camera cells",
            ));
        }
        Ok(())
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

/// A preview value and its authored spatial demand.
#[cfg(feature = "editor")]
pub(crate) struct ViewGraph {
    pub request: crate::gpu::view::request::SourceRequest,
    pub processed: Value,
}

#[cfg(feature = "editor")]
impl super::request::PhotographRequest {
    pub fn original(
        &self,
        device: &wgpu::Device,
        renderer: &crate::renderer::GpuRenderer,
        scan: &crate::scan::Scan,
        field: Option<Arc<crate::gpu::flat::GpuFlatField>>,
        target: crate::rendering::OutputTarget,
    ) -> Result<Value, Error> {
        self.original_output(
            device,
            renderer,
            scan,
            field,
            crate::rendering::OutputReference::DisplayReferred(target),
        )
    }

    pub(crate) fn output(
        &self,
        renderer: &crate::renderer::GpuRenderer,
        image: Value,
        reference: crate::rendering::OutputReference,
    ) -> Result<Value, Error> {
        compose_output(renderer, image, reference, self.luts())
    }

    pub(crate) fn original_output(
        &self,
        device: &wgpu::Device,
        renderer: &crate::renderer::GpuRenderer,
        scan: &crate::scan::Scan,
        field: Option<Arc<crate::gpu::flat::GpuFlatField>>,
        reference: crate::rendering::OutputReference,
    ) -> Result<Value, Error> {
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
                )?
                .processed(&device.limits())?
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
                    reference,
                    &input.pixel_pipeline(),
                );
            }
        };
        self.output(renderer, photograph, reference)
    }

    pub fn validate(&self, limits: &wgpu::Limits) -> Result<(), Error> {
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
                )
                .map(|_| ())
            }
            Self::Inspection(_) => Ok(()),
            #[cfg(test)]
            Self::Fixture(input) => input.validate_controls(limits),
        }
    }

    pub fn source_request(
        &self,
        frame: crate::frame_raster::FrameRaster,
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
        Ok(request)
    }

    pub fn has_repairs(&self) -> bool {
        match self {
            Self::Recipe(recipe) | Self::BeforeLooks(recipe) => {
                recipe.repairs.iter().any(|repair| repair.enabled)
            }
            Self::Inspection(_) => false,
            #[cfg(test)]
            Self::Fixture(input) => !input.repairs.is_empty() || !input.replacements.is_empty(),
        }
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
        matrix: glam::DMat3,
        quality: Quality,
    ) -> Result<super::NeutralInput, Error> {
        use crate::gpu::interpretation::SourceInterpretation;
        let repairs = match self {
            Self::Recipe(recipe) | Self::BeforeLooks(recipe) if quality.includes_repairs() => {
                recipe.repairs.clone()
            }
            #[cfg(test)]
            Self::Fixture(input) => input.repairs.clone(),
            _ => Vec::new(),
        };
        #[cfg(test)]
        let replacements = match self {
            Self::Fixture(input) => input.replacements.clone(),
            _ => Vec::new(),
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
            Self::Inspection(_) => SourceInterpretation::Inspection(matrix),
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
            #[cfg(test)]
            replacements,
        })
    }

    pub fn sampled_graph(
        &self,
        limits: &wgpu::Limits,
        source: &ProcessingSource,
        field: Option<Arc<GpuFlatField>>,
        spec: crate::gpu::view::request::SourceRequest,
    ) -> Result<ViewGraph, Error> {
        let (mut request, baseline) = self.sampled_prefix(source, field, spec)?;
        let (processed, plan) = self.filtered(limits, &request, baseline)?;
        request.plan = Arc::new(plan);
        Ok(ViewGraph { request, processed })
    }

    pub fn retained_graph(
        &self,
        limits: &wgpu::Limits,
        baseline: &Value,
        field: Option<Arc<GpuFlatField>>,
        request: &crate::gpu::view::request::SourceRequest,
    ) -> Result<ViewGraph, Error> {
        let baseline = self.retained_prefix(baseline, field, request)?;
        let (processed, plan) = self.filtered(limits, request, baseline)?;
        let mut request = request.clone();
        request.plan = Arc::new(plan);
        Ok(ViewGraph { request, processed })
    }

    pub fn sampled_prefix(
        &self,
        source: &ProcessingSource,
        field: Option<Arc<GpuFlatField>>,
        spec: crate::gpu::view::request::SourceRequest,
    ) -> Result<(crate::gpu::view::request::SourceRequest, Value), Error> {
        let geometry = source
            .geometry()
            .ok_or_else(|| Error::Gpu("preview source has no geometry".into()))?;
        let input =
            self.neutral_input(field.clone(), source.camera_matrix(), Quality::CameraProxy)?;
        if !input.repairs.is_empty() {
            return Err(Error::Gpu(
                "repaired previews require original-coordinate acquisition".into(),
            ));
        }
        #[cfg(test)]
        if !input.replacements.is_empty() {
            return Err(Error::Gpu(
                "repaired previews require original-coordinate acquisition".into(),
            ));
        }
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
        Ok((spec.clone(), Self::sampled_domain(root, &spec)?))
    }

    fn sampled_domain(
        root: Value,
        spec: &crate::gpu::view::request::SourceRequest,
    ) -> Result<Value, Error> {
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
        reconstruct(root, output)
    }

    pub fn retained_prefix(
        &self,
        source: &Value,
        field: Option<Arc<GpuFlatField>>,
        spec: &crate::gpu::view::request::SourceRequest,
    ) -> Result<Value, Error> {
        let program = self.input_transform()?;
        let root = self
            .neutral_input(field, program.matrix, Quality::CameraProxy)?
            .recompose_resident(source)?;
        Self::sampled_domain(root, spec)
    }

    pub fn require_proxy(&self, proxy: &crate::scan::CameraProxy) -> Result<(), Error> {
        if !self.input_transform()?.matches_proxy(proxy)? {
            return Err(Error::Gpu(
                "camera proxy does not match thumbnail source raster".into(),
            ));
        }
        Ok(())
    }

    #[cfg(test)]
    fn filtered_fixture(
        limits: &wgpu::Limits,
        spec: &crate::gpu::view::request::SourceRequest,
        neutral: Value,
        input: &AuthoredPhotograph,
    ) -> Result<(Value, crate::acquisition::Plan), Error> {
        let presentation = spec.frame();
        if spec.is_inspection() {
            let parameters = input.parameters();
            let matrix = glam::DMat3::from_cols(
                parameters.post[0].truncate().as_dvec3(),
                parameters.post[1].truncate().as_dvec3(),
                parameters.post[2].truncate().as_dvec3(),
            );
            let expected = ProcessingParameters::source_with_matrix(matrix)?;
            if input.grade().is_some()
                || !matrix.is_finite()
                || matrix.determinant() == 0.
                || bytemuck::bytes_of(&expected) != bytemuck::bytes_of(&parameters)
            {
                return Err(Error::Gpu(
                    "physical-view source inspection identity differs".into(),
                ));
            }
            let root = crate::gpu::photograph::PhotographGraph::inspection_view(
                neutral,
                parameters,
                presentation,
            )?;
            return Ok((root, spec.plan.as_ref().clone()));
        }
        let program = input.source_transform().ok_or_else(|| {
            Error::Gpu("positive view requires captured source interpretation".into())
        })?;
        if crate::gpu::photograph::PhotographGraph::input_transform(&neutral)?
            .is_some_and(|captured| captured != *program)
        {
            return Err(Error::Gpu("physical-view source is stale".into()));
        }
        input.grade().unwrap().require_extent(spec.area)?;
        input.validate_controls(limits)?;
        let stages = input.pixel_pipeline();
        let plan = spec.processing_plan(&stages)?;
        let grading = input
            .grade()
            .unwrap()
            .for_grid(neutral.domain().grid.coverage);
        let root = crate::gpu::photograph::PhotographGraph::filtered(
            &grading,
            neutral,
            &stages,
            presentation,
            plan.output(),
        )?;
        Ok((root, plan))
    }

    pub fn filtered(
        &self,
        limits: &wgpu::Limits,
        spec: &crate::gpu::view::request::SourceRequest,
        neutral: crate::gpu::evaluation::Value,
    ) -> Result<(crate::gpu::evaluation::Value, crate::acquisition::Plan), Error> {
        match self {
            Self::Recipe(recipe) | Self::BeforeLooks(recipe) => {
                super::PhotographGraph::filtered_recipe(limits, spec, neutral, recipe)
            }
            Self::Inspection(view) => {
                if neutral.domain().meaning != crate::gpu::evaluation::Meaning::Inspection {
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
            Self::Fixture(input) => Self::filtered_fixture(limits, spec, neutral, input),
        }
    }

    pub fn source_graphs(
        &self,
        scan: &crate::scan::Scan,
        field: Option<Arc<crate::gpu::flat::GpuFlatField>>,
        specs: &[crate::gpu::view::request::SourceRequest],
    ) -> Result<Vec<crate::gpu::evaluation::Value>, Error> {
        let source = crate::gpu::evaluation::camera_source(scan)?;
        specs
            .iter()
            .map(|spec| {
                let input =
                    self.neutral_input(field.clone(), scan.camera_to_ap1, Quality::PhysicalView)?;
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
    pub fn processing(&self, limits: &wgpu::Limits) -> Result<AuthoredPhotograph, Error> {
        let input = match self {
            Self::Recipe(recipe) | Self::BeforeLooks(recipe) => {
                AuthoredPhotograph::from_recipe(limits, recipe)?
            }
            Self::Inspection(view) => AuthoredPhotograph::from(view.parameters()?),
            #[cfg(test)]
            Self::Fixture(input) => input.clone(),
        };
        Ok(input)
    }
}
