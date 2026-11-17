//! Purpose-specific preview authoring; presentation consumers request the resulting endpoint.

use super::*;
use crate::gpu::photograph::request::PhotographRequest;
use crate::gpu::view::request::SourceRequest;
use crate::rendering::{OutputReference, OutputTarget};

impl PhotographRequest {
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
