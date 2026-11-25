//! Select a photograph or its source-view branch; prepared arguments stay in graph operations.

use super::view_source::SourceRequest;
use super::*;
use crate::acquisition::Sampling;
use crate::acquisition::camera::camera_source;
use crate::image_recipe::{ImageRecipe, quality::Quality};
use crate::looks::LutChain;
use crate::photographic::parameters::ProcessingParameters;
use crate::photographic::stages::interpretation::InspectionView;
use crate::photographic::stages::{display_rendering as drt, look::Looks};
#[cfg(test)]
use crate::photographic::test_inputs::AuthoredPhotograph;

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
                if !crate::photographic::stages::interpretation::InputTransform::from_recipe(
                    &recipe.neutral,
                )?
                .matches_native(scan)?
                {
                    return Err(Error::Gpu(
                        "recipe interpretation differs from captured source".into(),
                    ));
                }
                composition::processed_originals(limits, camera_source(scan)?, field, recipe)?
            }
            Self::Inspection(view) => composition::inspection_original(
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
        let frame = request.frame();
        let layout = match self {
            Self::Recipe(recipe) => composition::plan_sampling(recipe, frame, sampling)?,
            Self::Inspection(_) => {
                super::sampling::ViewSampling::new(frame, None, sampling, |input| {
                    let graded = Grading::roll_image(input, Default::default())?;
                    super::composition::film_effects(graded, None, None, 0, None)
                })?
            }
            #[cfg(test)]
            Self::Fixture(_) => request.definition().view_sampling(frame, sampling)?,
        };
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

    pub fn input_transform(
        &self,
    ) -> Result<crate::photographic::stages::interpretation::InputTransform, Error> {
        match self {
            Self::Recipe(recipe) => {
                crate::photographic::stages::interpretation::InputTransform::from_recipe(
                    &recipe.neutral,
                )
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

    fn neutral(
        &self,
        source: Value,
        field: Option<Arc<GpuFlatField>>,
        mapping: Mapping,
        matrix: glam::DMat3,
        continuous: bool,
        quality: Quality,
    ) -> Result<Value, Error> {
        let (program, repairs) = match self {
            Self::Recipe(recipe) => {
                composition::require_field(field.as_ref(), recipe)?;
                let repairs = if quality.includes_repairs() {
                    recipe.repairs.as_slice()
                } else {
                    &[]
                };
                (Some(self.input_transform()?), repairs)
            }
            Self::Inspection(_) => (None, &[][..]),
            #[cfg(test)]
            Self::Fixture(input) => (input.source_transform().copied(), input.repairs.as_slice()),
        };
        let (matrix, interpretation) = if let Some(program) = program {
            (program.matrix(), Interpretation::Program(program))
        } else {
            (
                matrix,
                Interpretation::Packed(ProcessingParameters::source()),
            )
        };
        #[cfg(test)]
        let replacements = match self {
            Self::Fixture(input) => input.replacements.as_slice(),
            _ => &[],
        };
        validate_camera_range(matrix, field.as_deref(), continuous)?;
        let camera = camera_image(source, field, mapping);
        let unrepaired = composition::interpret_camera(camera, interpretation, matrix)?;
        composition::repair_neutral(
            unrepaired,
            mapping,
            repairs,
            #[cfg(test)]
            replacements,
        )
    }

    pub(crate) fn original_neutral(
        &self,
        source: Value,
        field: Option<Arc<GpuFlatField>>,
        matrix: glam::DMat3,
    ) -> Result<Value, Error> {
        if source.domain().grid != Grid::axis_aligned([0.; 2], [1.; 2]) {
            return Err(Error::InvalidParameter(
                "neutral conversion requires original camera cells",
            ));
        }
        self.neutral(
            source,
            field,
            Mapping::Originals,
            matrix,
            true,
            Quality::Originals,
        )
    }

    pub fn sampled_view(
        &self,
        limits: &wgpu::Limits,
        source: &ProcessingSource,
        field: Option<Arc<GpuFlatField>>,
        spec: SourceRequest,
    ) -> Result<(Value, SourceRequest), Error> {
        let baseline = self.sampled_prefix(source, field, &spec)?;
        let (processed, plan) = self.filtered(limits, &spec, baseline)?;
        Ok((processed, spec.with_plan(plan)))
    }

    pub fn retained_view(
        &self,
        limits: &wgpu::Limits,
        baseline: &Value,
        field: Option<Arc<GpuFlatField>>,
        request: &SourceRequest,
    ) -> Result<(Value, SourceRequest), Error> {
        let baseline = self.retained_prefix(baseline, field, request)?;
        let (processed, plan) = self.filtered(limits, request, baseline)?;
        Ok((processed, request.clone().with_plan(plan)))
    }

    pub fn sampled_prefix(
        &self,
        source: &ProcessingSource,
        field: Option<Arc<GpuFlatField>>,
        spec: &SourceRequest,
    ) -> Result<Value, Error> {
        let geometry = source
            .geometry()
            .ok_or_else(|| Error::Gpu("preview source has no geometry".into()))?;
        #[cfg(test)]
        if let Self::Fixture(input) = self
            && (!input.repairs.is_empty() || !input.replacements.is_empty())
        {
            return Err(Error::Gpu(
                "repaired previews require original-coordinate acquisition".into(),
            ));
        }
        let raster = crate::acquisition::raster::Raster::region(geometry.region(), geometry.size());
        let codes = resident::codes(source, raster)?;
        let root = if let Some(recipe) = self.recipe() {
            composition::sampled_unrepaired(codes, source, field, recipe)?
        } else {
            self.neutral(
                codes,
                field,
                Mapping::Resident(Some(geometry)),
                source.camera_matrix(),
                source.continuous,
                Quality::CameraProxy,
            )?
        };
        Self::sampled_domain(root, spec)
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
        let (codes, mapping, continuous) = resident_camera_argument(source)?;
        let root = self.neutral(
            codes.clone(),
            field,
            mapping,
            program.matrix(),
            continuous,
            Quality::CameraProxy,
        )?;
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
            let root = composition::inspection_view(neutral, parameters, presentation)?;
            return Ok((root, spec.plan().clone()));
        }
        let program = input.source_transform().ok_or_else(|| {
            Error::Gpu("positive view requires captured source interpretation".into())
        })?;
        if input_transform(&neutral)?.is_some_and(|captured| captured != *program) {
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
        let root = authoring::filtered(&grading, neutral, &stages, presentation, plan.output())?;
        Ok((root, plan))
    }

    pub fn filtered(
        &self,
        limits: &wgpu::Limits,
        spec: &SourceRequest,
        neutral: crate::evaluation::Value,
    ) -> Result<(crate::evaluation::Value, crate::acquisition::Plan), Error> {
        match self {
            Self::Recipe(recipe) => composition::filtered_recipe(limits, spec, neutral, recipe),
            Self::Inspection(view) => Ok((
                composition::inspection_view(neutral, view.parameters()?, spec.frame())?,
                spec.plan().clone(),
            )),
            #[cfg(test)]
            Self::Fixture(input) => Self::filtered_fixture(limits, spec, neutral, input),
        }
    }

    pub(crate) fn source_view(
        &self,
        scan: &crate::scan::Scan,
        field: Option<Arc<GpuFlatField>>,
        spec: &SourceRequest,
    ) -> Result<Value, Error> {
        let neutral = self.original_neutral(camera_source(scan)?, field, scan.camera_to_ap1)?;
        let mut grid = Grid::from_frame(spec.plan().output());
        grid.coverage = 8;
        crate::reconstruction::reconstruct(neutral, Domain { bounds: None, grid })
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
                composition::processed_resident(limits, source, layout, field, recipe)?
            }
            Self::Inspection(view) => {
                let raster = resident::raster(source, layout)?;
                resident::inspection(resident::codes(source, raster)?, source, field, *view)?
            }
            #[cfg(test)]
            Self::Fixture(input) => {
                let raster = resident::raster(source, layout)?;
                let codes = resident::codes(source, raster)?;
                if let Some(program) = input.source_transform() {
                    input.definition().compose(resident::photograph(
                        limits,
                        codes,
                        source,
                        field,
                        *program,
                        input.grade().expect("positive grading").clone(),
                        &input.repairs,
                    )?)?
                } else {
                    resident::inspection(
                        codes,
                        source,
                        field,
                        crate::photographic::stages::interpretation::InspectionView::from_parameters(
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
        let (processed, spec) = self.sampled_view(limits, source, field, spec)?;
        Ok((self.display(processed, looks, target)?, spec))
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
        let (processed, spec) = self.retained_view(limits, source, field, spec)?;
        Ok((self.display(processed, looks, target)?, spec))
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
                composition::preview(limits, scan, field, recipe, spec.frame())?
            }
            #[cfg(test)]
            Self::Fixture(input) if input.is_positive() => {
                let (graph, plan) = authoring::PhotographTaps::preview_with_input(
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
                let source = self.source_view(scan, field, &spec)?;
                self.filtered(limits, &spec, source)?
            }
        };
        let spec = spec.with_plan(plan);
        Ok((self.display(image, looks, target)?, spec))
    }
}
