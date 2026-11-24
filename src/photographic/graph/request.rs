//! Select a photograph or its source-view branch; prepared arguments stay in graph operations.

use super::view_source::SourceRequest;
use super::*;
use crate::acquisition::Sampling;
use crate::gpu::interpretation::InputTransform;
use crate::gpu::interpretation::InspectionView;
use crate::gpu::parameters::ProcessingParameters;
use crate::gpu::source::camera_source;
#[cfg(test)]
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::gpu::{drt, lut3d::Looks};
use crate::image_recipe::{ImageRecipe, quality::Quality};
use crate::looks::LutChain;

static NO_LUTS: LutChain = LutChain {
    lut: None,
    film_and_paper: None,
};

#[derive(Clone, PartialEq)]
pub(crate) enum PhotographRequest {
    Recipe(Arc<ImageRecipe>),

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
            Self::Recipe(recipe) => Some(recipe),
            _ => None,
        }
    }

    pub fn is_inspection(&self) -> bool {
        match self {
            Self::Recipe(_) => false,
            Self::Inspection(_) => true,
            #[cfg(test)]
            Self::Fixture(input) => !input.is_positive(),
        }
    }

    pub fn luts(&self) -> &LutChain {
        match self {
            Self::Recipe(recipe) => &recipe.luts,
            Self::Inspection(_) => &NO_LUTS,
            #[cfg(test)]
            Self::Fixture(input) => input.luts(),
        }
    }

    #[cfg(all(test, feature = "ui-tests"))]
    pub fn look_id(&self) -> Option<crate::looks::LookId> {
        self.luts().id()
    }
}

/// A preview value and its authored spatial demand.
pub(crate) struct ViewGraph {
    pub request: SourceRequest,
    pub processed: Value,
}

impl PhotographRequest {
    pub(crate) fn display(
        &self,
        image: Value,
        looks: &Looks,
        target: &drt::Target,
    ) -> Result<Value, Error> {
        display_output(image, self.luts(), looks, target)
    }

    pub fn original(
        &self,
        limits: &wgpu::Limits,
        scan: &crate::scan::Scan,
        field: Option<Arc<GpuFlatField>>,
        looks: &Looks,
        target: &drt::Target,
    ) -> Result<Value, Error> {
        if scan.flat_field() != field.as_ref().map(|field| field.field()) {
            return Err(Error::Gpu("CPU and GPU flat-field resources differ".into()));
        }
        let photograph = match self {
            Self::Recipe(recipe) => {
                recipe.neutral.require_scan(scan)?;
                if !crate::gpu::interpretation::InputTransform::from_recipe(&recipe.neutral)?
                    .matches_native(scan)?
                {
                    return Err(Error::Gpu(
                        "recipe interpretation differs from captured source".into(),
                    ));
                }
                super::RecipeGraph::originals(camera_source(scan)?, field, recipe)?
                    .processed(limits)?
            }
            Self::Inspection(view) => super::PhotographGraph::inspection_original(
                camera_source(scan)?,
                field,
                scan.camera_to_ap1,
                *view,
            )?,
            #[cfg(test)]
            Self::Fixture(input) => {
                return super::authoring::original_chain(
                    limits,
                    scan,
                    field,
                    input,
                    &input.definition(),
                    looks,
                    target,
                );
            }
        };
        self.display(photograph, looks, target)
    }

    pub fn validate(&self, limits: &wgpu::Limits) -> Result<(), Error> {
        match self {
            Self::Recipe(recipe) => {
                let program = self.input_transform()?;
                super::admission::prepare(
                    limits,
                    program.input(),
                    recipe.roll,
                    &recipe.grade,
                    program.matrix(),
                    recipe.neutral.source_development.active_area().into(),
                )
                .map(|_| ())
            }
            Self::Inspection(_) => Ok(()),
            #[cfg(test)]
            Self::Fixture(input) => input.validate_controls(limits),
        }
    }

    pub(crate) fn plan_sampling(
        &self,
        request: SourceRequest,
        sampling: Sampling,
    ) -> Result<SourceRequest, Error> {
        let assembly = match self {
            Self::Recipe(recipe) => {
                InputTransform::from_recipe(&recipe.neutral)?.neutral_parameters()?;
                RecipeGraph::view_definition(&wgpu::Limits::default(), recipe)?
            }
            Self::Inspection(_) => Assembly::default(),
            #[cfg(test)]
            Self::Fixture(_) => request.definition().clone(),
        };
        let layout = assembly.view_sampling(request.frame(), sampling)?;
        Ok(request.with_layout(layout))
    }

    #[cfg(test)]
    pub fn source_request(
        &self,
        frame: crate::frame_raster::FrameRaster,
    ) -> Result<SourceRequest, Error> {
        match self {
            Self::Recipe(_) => self.plan_sampling(
                SourceRequest::for_frame(frame)?,
                Sampling::NeutralFootprints,
            ),
            Self::Fixture(input) if input.is_positive() => {
                SourceRequest::for_processing(frame, input)
            }
            Self::Inspection(_) | Self::Fixture(_) => SourceRequest::for_frame(frame),
        }
    }

    pub fn input_transform(&self) -> Result<crate::gpu::interpretation::InputTransform, Error> {
        match self {
            Self::Recipe(recipe) => {
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
            Self::Recipe(recipe) if quality.includes_repairs() => recipe.repairs.clone(),
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
            Self::Recipe(recipe) => {
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
        spec: SourceRequest,
    ) -> Result<ViewGraph, Error> {
        let (request, baseline) = self.sampled_prefix(source, field, spec)?;
        let (processed, plan) = self.filtered(limits, &request, baseline)?;
        let request = request.with_plan(plan);
        Ok(ViewGraph { request, processed })
    }

    pub fn retained_graph(
        &self,
        limits: &wgpu::Limits,
        baseline: &Value,
        field: Option<Arc<GpuFlatField>>,
        request: &SourceRequest,
    ) -> Result<ViewGraph, Error> {
        let baseline = self.retained_prefix(baseline, field, request)?;
        let (processed, plan) = self.filtered(limits, request, baseline)?;
        let request = request.clone().with_plan(plan);
        Ok(ViewGraph { request, processed })
    }

    pub fn sampled_prefix(
        &self,
        source: &ProcessingSource,
        field: Option<Arc<GpuFlatField>>,
        spec: SourceRequest,
    ) -> Result<(SourceRequest, Value), Error> {
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

    fn sampled_domain(root: Value, spec: &SourceRequest) -> Result<Value, Error> {
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
            },
        };
        crate::reconstruction::reconstruct(root, output)
    }

    pub fn retained_prefix(
        &self,
        source: &Value,
        field: Option<Arc<GpuFlatField>>,
        spec: &SourceRequest,
    ) -> Result<Value, Error> {
        let program = self.input_transform()?;
        let root = self
            .neutral_input(field, program.matrix(), Quality::CameraProxy)?
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
        spec: &SourceRequest,
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
            let root = crate::photographic::graph::PhotographGraph::inspection_view(
                neutral,
                parameters,
                presentation,
            )?;
            return Ok((root, spec.plan().clone()));
        }
        let program = input.source_transform().ok_or_else(|| {
            Error::Gpu("positive view requires captured source interpretation".into())
        })?;
        if crate::photographic::graph::PhotographGraph::input_transform(&neutral)?
            .is_some_and(|captured| captured != *program)
        {
            return Err(Error::Gpu("physical-view source is stale".into()));
        }
        input.grade().unwrap().require_extent(spec.area)?;
        input.validate_controls(limits)?;
        let stages = input.definition();
        let plan = stages.processing_plan(spec)?;
        let grading = input
            .grade()
            .unwrap()
            .for_grid(neutral.domain().grid.coverage);
        let root = crate::photographic::graph::PhotographGraph::filtered(
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
        spec: &SourceRequest,
        neutral: crate::gpu::evaluation::Value,
    ) -> Result<(crate::gpu::evaluation::Value, crate::acquisition::Plan), Error> {
        match self {
            Self::Recipe(recipe) => {
                super::PhotographGraph::filtered_recipe(limits, spec, neutral, recipe)
            }
            Self::Inspection(view) => Ok((
                super::PhotographGraph::inspection_view(neutral, view.parameters()?, spec.frame())?,
                spec.plan().clone(),
            )),
            #[cfg(test)]
            Self::Fixture(input) => Self::filtered_fixture(limits, spec, neutral, input),
        }
    }

    pub fn source_graphs(
        &self,
        scan: &crate::scan::Scan,
        field: Option<Arc<crate::gpu::flat::GpuFlatField>>,
        specs: &[SourceRequest],
    ) -> Result<Vec<crate::gpu::evaluation::Value>, Error> {
        let source = camera_source(scan)?;
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
            Self::Recipe(recipe) => AuthoredPhotograph::from_recipe(limits, recipe)?,
            Self::Inspection(view) => AuthoredPhotograph::from(view.parameters()?),
            #[cfg(test)]
            Self::Fixture(input) => input.clone(),
        };
        Ok(input)
    }

    pub(crate) fn resident_processed(
        &self,
        limits: &wgpu::Limits,
        source: &ProcessingSource,
        layout: [u32; 2],
        field: Option<Arc<GpuFlatField>>,
    ) -> Result<Value, Error> {
        let image = match self {
            Self::Recipe(recipe) => {
                let raster = RecipeGraph::sampled_raster(source, recipe, layout)?;
                RecipeGraph::sampled_camera(
                    PhotographGraph::resident_codes(source, raster)?,
                    source,
                    field,
                    recipe,
                )?
                .processed(limits)?
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
                            limits,
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
        limits: &wgpu::Limits,
        source: &ProcessingSource,
        layout: [u32; 2],
        field: Option<Arc<GpuFlatField>>,
        looks: &Looks,
        target: &drt::Target,
    ) -> Result<Value, Error> {
        self.display(
            self.resident_processed(limits, source, layout, field)?,
            looks,
            target,
        )
    }

    pub(crate) fn sampled_output(
        &self,
        limits: &wgpu::Limits,
        source: &ProcessingSource,
        field: Option<Arc<GpuFlatField>>,
        spec: SourceRequest,
        looks: &Looks,
        target: &drt::Target,
    ) -> Result<(Value, SourceRequest), Error> {
        let graph = self.sampled_graph(limits, source, field, spec)?;
        let image = self.display(graph.processed, looks, target)?;
        Ok((image, graph.request))
    }

    pub(crate) fn retained_output(
        &self,
        limits: &wgpu::Limits,
        source: &Value,
        field: Option<Arc<GpuFlatField>>,
        spec: &SourceRequest,
        looks: &Looks,
        target: &drt::Target,
    ) -> Result<(Value, SourceRequest), Error> {
        let graph = self.retained_graph(limits, source, field, spec)?;
        let image = self.display(graph.processed, looks, target)?;
        Ok((image, graph.request))
    }

    pub(crate) fn preview_output(
        &self,
        limits: &wgpu::Limits,
        scan: &crate::scan::Scan,
        field: Option<Arc<GpuFlatField>>,
        spec: SourceRequest,
        looks: &Looks,
        target: &drt::Target,
    ) -> Result<(Value, SourceRequest), Error> {
        let (image, plan) = match self {
            Self::Recipe(recipe) => {
                let (graph, plan) =
                    PhotographGraph::preview(limits, scan, field, recipe, spec.frame(), None)?;
                (graph.processed, plan)
            }
            #[cfg(test)]
            Self::Fixture(input) if input.is_positive() => {
                let (graph, plan) = PhotographGraph::preview_with_input(
                    limits,
                    camera_source(scan)?,
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
                self.filtered(limits, &spec, source)?
            }
        };
        let spec = spec.with_plan(plan);
        Ok((self.display(image, looks, target)?, spec))
    }
}
