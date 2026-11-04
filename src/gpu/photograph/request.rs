//! Select a photograph or its source-view branch; prepared arguments stay in graph operations.

use crate::Error;
use crate::gpu::interpretation::InspectionView;
use crate::gpu::parameters::ProcessingParameters;
#[cfg(test)]
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::image_recipe::ImageRecipe;
use crate::looks::LutChain;
use std::sync::Arc;

static NO_LUTS: LutChain = LutChain {
    lut: None,
    film_and_paper: None,
};

#[derive(Clone)]
pub(crate) enum PhotographRequest {
    Recipe(Arc<ImageRecipe>),
    Inspection(InspectionView),
    #[cfg(test)]
    Fixture(AuthoredPhotograph),
}

impl PartialEq for PhotographRequest {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Recipe(a), Self::Recipe(b)) => a.same_photographic(b) && a.luts == b.luts,
            (Self::Inspection(a), Self::Inspection(b)) => a == b,
            #[cfg(test)]
            (Self::Fixture(a), Self::Fixture(b)) => a == b,
            _ => false,
        }
    }
}

impl PhotographRequest {
    pub fn inspection(parameters: ProcessingParameters) -> Result<Self, Error> {
        Ok(Self::Inspection(InspectionView::from_parameters(
            parameters,
        )?))
    }

    pub fn endpoint(&self) -> Result<super::Endpoint, Error> {
        Ok(match self {
            Self::Recipe(_) => super::Endpoint::Processed,
            Self::Inspection(view) => super::Endpoint::Inspection(*view),
            #[cfg(test)]
            Self::Fixture(input) if !input.is_positive() => {
                super::Endpoint::Inspection(InspectionView::from_parameters(input.parameters())?)
            }
            #[cfg(test)]
            Self::Fixture(_) => super::Endpoint::Processed,
        })
    }

    pub fn is_inspection(&self) -> bool {
        match self {
            Self::Recipe(_) => false,
            Self::Inspection(_) => true,
            #[cfg(test)]
            Self::Fixture(input) => !input.is_positive(),
        }
    }

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
            Self::Recipe(recipe) => {
                recipe.neutral.require_scan(scan)?;
                if !crate::gpu::interpretation::InputTransform::from_recipe(&recipe.neutral)?
                    .matches_native(scan)?
                {
                    return Err(Error::Gpu(
                        "recipe interpretation differs from captured source".into(),
                    ));
                }
                super::PhotographGraph::recipe_at(
                    &device.limits(),
                    crate::gpu::evaluation::camera_source(scan)?,
                    field,
                    recipe,
                    repairs,
                    self.endpoint()?,
                )?
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

    pub fn luts(&self) -> &LutChain {
        match self {
            Self::Recipe(recipe) => &recipe.luts,
            Self::Inspection(_) => &NO_LUTS,
            #[cfg(test)]
            Self::Fixture(input) => input.luts(),
        }
    }

    pub fn look_id(&self) -> Option<crate::looks::LookId> {
        self.luts().id()
    }

    pub fn validate(
        &self,
        limits: &wgpu::Limits,
        repairs: &crate::gpu::repairs::Snapshot,
    ) -> Result<(), Error> {
        match self {
            Self::Recipe(recipe) => {
                let program = self.input_transform()?;
                super::admission::prepare(
                    limits,
                    program.input,
                    recipe.roll,
                    &recipe.grade,
                    program.matrix,
                    recipe.neutral.source_development.active_area.into(),
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
            Self::Recipe(recipe) => SourceRequest::for_recipe(frame, recipe)?,
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
        repairs: crate::gpu::repairs::Snapshot,
        matrix: glam::DMat3,
    ) -> Result<super::NeutralInput, Error> {
        use crate::gpu::interpretation::SourceInterpretation;
        let interpretation = match self {
            Self::Recipe(recipe) => {
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
            Self::Recipe(recipe) => {
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
                let input =
                    self.neutral_input(field.clone(), spec.repairs.clone(), scan.camera_to_ap1)?;
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
            Self::Recipe(recipe) => AuthoredPhotograph::from_recipe(limits, recipe)?,
            Self::Inspection(view) => AuthoredPhotograph::from(view.parameters()?),
            #[cfg(test)]
            Self::Fixture(input) => input.clone(),
        };
        input.repairs = repairs.clone();
        Ok(input)
    }
}
