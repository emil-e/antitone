//! Select a photograph or its source-view branch; prepared arguments stay in graph operations.

use super::*;
use crate::gpu::interpretation::InputTransform;
use crate::gpu::interpretation::InspectionView;
use crate::gpu::parameters::ProcessingParameters;
#[cfg(test)]
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::gpu::view::request::SourceRequest;
use crate::image_recipe::{ImageRecipe, quality::Quality};
use crate::looks::LutChain;
use crate::rendering::{OutputReference, OutputTarget};

static NO_LUTS: LutChain = LutChain {
    lut: None,
    film_and_paper: None,
};

#[derive(Clone, PartialEq)]
pub(crate) enum PhotographRequest {
    Recipe(Arc<ImageRecipe>),
    BeforeLooks(Arc<ImageRecipe>),
    Inspection(InspectionView),
    #[cfg(test)]
    Fixture(AuthoredPhotograph),
}

impl PhotographRequest {
    pub fn inspection(parameters: ProcessingParameters) -> Result<Self, Error> {
        Ok(Self::Inspection(InspectionView::from_parameters(
            parameters,
        )?))
    }

    pub fn recipe(&self) -> Option<&Arc<ImageRecipe>> {
        match self {
            Self::Recipe(recipe) | Self::BeforeLooks(recipe) => Some(recipe),
            _ => None,
        }
    }

    pub fn is_inspection(&self) -> bool {
        match self {
            Self::Recipe(_) | Self::BeforeLooks(_) => false,
            Self::Inspection(_) => true,
            #[cfg(test)]
            Self::Fixture(input) => !input.is_positive(),
        }
    }

    pub fn luts(&self) -> &LutChain {
        match self {
            Self::Recipe(recipe) => &recipe.luts,
            Self::BeforeLooks(_) | Self::Inspection(_) => &NO_LUTS,
            #[cfg(test)]
            Self::Fixture(input) => input.luts(),
        }
    }

    pub fn look_id(&self) -> Option<crate::looks::LookId> {
        self.luts().id()
    }
}

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
        request.with_definition(RecipeGraph::view_definition(
            &wgpu::Limits::default(),
            recipe,
        )?)
    }
}

/// A preview value and its authored spatial demand.
pub(crate) struct ViewGraph {
    pub request: crate::gpu::view::request::SourceRequest,
    pub processed: Value,
}

impl PhotographRequest {
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
                    &input.definition(),
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

    pub(crate) fn planned_request(
        &self,
        limits: &wgpu::Limits,
        request: crate::gpu::view::request::SourceRequest,
        plan: crate::acquisition::Plan,
    ) -> Result<crate::gpu::view::request::SourceRequest, Error> {
        let definition = match self {
            Self::Recipe(recipe) | Self::BeforeLooks(recipe) => {
                RecipeGraph::view_definition(limits, recipe)?
            }
            Self::Inspection(_) => Assembly::default(),
            #[cfg(test)]
            Self::Fixture(input) => {
                let mut definition = input.definition();
                definition.grading = input.grade().cloned();
                definition
            }
        };
        Ok(request.with_plan(definition, plan))
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
        let (request, baseline) = self.sampled_prefix(source, field, spec)?;
        let (processed, plan) = self.filtered(limits, &request, baseline)?;
        let request = self.planned_request(limits, request, plan)?;
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
        let request = self.planned_request(limits, request.clone(), plan)?;
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
        let output = match spec.plan().sampling() {
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
                    ..Grid::from_frame(spec.plan().output())
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
            return Ok((root, spec.plan().clone()));
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
        let stages = input.definition();
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
                    spec.plan().clone(),
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
                    .original_views(source.clone(), [spec.plan().output()])?
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

    pub(crate) fn resident_processed(
        &self,
        renderer: &crate::renderer::GpuRenderer,
        source: &ProcessingSource,
        layout: [u32; 2],
        field: Option<Arc<GpuFlatField>>,
    ) -> Result<Value, Error> {
        let limits = renderer.evaluation().device().limits();
        let image = match self {
            Self::Recipe(recipe) | Self::BeforeLooks(recipe) => {
                let raster = RecipeGraph::sampled_raster(source, recipe, layout)?;
                RecipeGraph::sampled_camera(
                    PhotographGraph::resident_codes(source, raster)?,
                    source,
                    field,
                    recipe,
                )?
                .processed(&limits)?
            }
            Self::Inspection(view) => {
                let raster = PhotographGraph::resident_raster(source, layout)?;
                PhotographGraph::resident_inspection(
                    PhotographGraph::resident_codes(source, raster)?,
                    source,
                    field,
                    *view,
                )?
            }
            #[cfg(test)]
            Self::Fixture(input) => {
                let raster = PhotographGraph::resident_raster(source, layout)?;
                let codes = PhotographGraph::resident_codes(source, raster)?;
                if let Some(program) = input.source_transform() {
                    input.definition().compose(
                        PhotographGraph::resident_photograph(
                            &limits,
                            codes,
                            operations::CameraInput::resident(source, *program, field),
                            *program,
                            input.grade().expect("positive grading").clone(),
                            input.repairs.clone(),
                        )?
                        .processed,
                    )?
                } else {
                    PhotographGraph::resident_inspection(
                        codes,
                        source,
                        field,
                        crate::gpu::interpretation::InspectionView::from_parameters(
                            input.parameters(),
                        )?,
                    )?
                }
            }
        };
        Ok(image)
    }

    pub(crate) fn resident_output(
        &self,
        renderer: &crate::renderer::GpuRenderer,
        source: &ProcessingSource,
        layout: [u32; 2],
        field: Option<Arc<GpuFlatField>>,
        target: OutputTarget,
    ) -> Result<Value, Error> {
        self.output(
            renderer,
            self.resident_processed(renderer, source, layout, field)?,
            OutputReference::DisplayReferred(target),
        )
    }

    pub(crate) fn sampled_output(
        &self,
        renderer: &crate::renderer::GpuRenderer,
        source: &ProcessingSource,
        field: Option<Arc<GpuFlatField>>,
        spec: SourceRequest,
        target: OutputTarget,
    ) -> Result<(Value, SourceRequest), Error> {
        let graph = self.sampled_graph(
            &renderer.evaluation().device().limits(),
            source,
            field,
            spec,
        )?;
        let image = self.output(
            renderer,
            graph.processed,
            OutputReference::DisplayReferred(target),
        )?;
        Ok((image, graph.request))
    }

    pub(crate) fn retained_output(
        &self,
        renderer: &crate::renderer::GpuRenderer,
        source: &Value,
        field: Option<Arc<GpuFlatField>>,
        spec: &SourceRequest,
        target: OutputTarget,
    ) -> Result<(Value, SourceRequest), Error> {
        let graph = self.retained_graph(
            &renderer.evaluation().device().limits(),
            source,
            field,
            spec,
        )?;
        let image = self.output(
            renderer,
            graph.processed,
            OutputReference::DisplayReferred(target),
        )?;
        Ok((image, graph.request))
    }

    pub(crate) fn preview_output(
        &self,
        renderer: &crate::renderer::GpuRenderer,
        scan: &crate::scan::Scan,
        field: Option<Arc<GpuFlatField>>,
        spec: SourceRequest,
        target: OutputTarget,
    ) -> Result<(Value, SourceRequest), Error> {
        let limits = renderer.evaluation().device().limits();
        let (image, plan) = match self {
            Self::Recipe(recipe) | Self::BeforeLooks(recipe) => {
                let (graph, plan) =
                    PhotographGraph::preview(&limits, scan, field, recipe, spec.frame(), None)?;
                (graph.processed, plan)
            }
            #[cfg(test)]
            Self::Fixture(input) if input.is_positive() => {
                let (graph, plan) = PhotographGraph::preview_with_input(
                    &limits,
                    crate::gpu::evaluation::camera_source(scan)?,
                    field,
                    input,
                    spec.frame(),
                    None,
                )?;
                (graph.processed, plan)
            }
            _ => {
                let source = self
                    .source_graphs(scan, field, std::slice::from_ref(&spec))?
                    .remove(0);
                self.filtered(&limits, &spec, source)?
            }
        };
        let spec = self.planned_request(&limits, spec, plan)?;
        Ok((
            self.output(renderer, image, OutputReference::DisplayReferred(target))?,
            spec,
        ))
    }
}
