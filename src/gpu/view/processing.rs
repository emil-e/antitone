//! Captured filtered-AP1 composition and recording against its supplied values.

use super::storage::float_buffer;
use crate::Error;
use crate::gpu::grading::Grading;
use crate::gpu::input::ProcessingInput;
use crate::gpu::parameters::ProcessingParameters;
use crate::gpu::view::acquisition::AcquiredViewRaster;
use crate::gpu::view::request::SourceRequest;
use glam::DMat3;
use std::sync::Arc;
use std::sync::Mutex;

#[cfg(test)]
mod tests;

// Required working storage is admitted independently of generated-value retention.
// Large presentations can keep their grid without an implicit coarse-quality fallback.
const VIEW_SCRATCH_BYTES: u64 = 1280 << 20;

pub struct ProcessedView {
    pub source: Arc<AcquiredViewRaster>,
    pub grade: Option<Arc<Grading>>,
    pub plan: crate::acquisition::Plan,
    pub definition: Arc<crate::gpu::evaluation::ImageNode>,
    pub stages: Option<crate::gpu::pixel_pipeline::Prepared>,
    pub pixels: crate::gpu::evaluation::BufferImage,
    pub submitted: Mutex<bool>,
}

impl ProcessedView {
    pub fn encode(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder) {
        let Some(stages) = &self.stages else {
            return;
        };
        debug_assert_eq!(stages.raster.size(), self.plan.input().size());
        stages
            .encode_from(device, encoder, &self.source.pixels, &self.pixels)
            .expect("validated photographic composition");
    }

    fn supplies(
        &self,
        definition: &Arc<crate::gpu::evaluation::ImageNode>,
        source: &AcquiredViewRaster,
    ) -> bool {
        match (self.definition.identity(), definition.identity()) {
            (Some(a), Some(b)) => a == b,
            _ => self.source.pixels == source.pixels && self.definition.same_definition(definition),
        }
    }

    pub fn matches(&self, limits: &wgpu::Limits, input: &ProcessingInput) -> bool {
        Self::definition(
            limits,
            &self.source.spec,
            self.source.definition.clone(),
            input,
        )
        .is_ok_and(|(root, _)| self.supplies(&root, &self.source))
    }

    pub fn definition(
        limits: &wgpu::Limits,
        spec: &SourceRequest,
        neutral: Arc<crate::gpu::evaluation::ImageNode>,
        input: &ProcessingInput,
    ) -> Result<
        (
            Arc<crate::gpu::evaluation::ImageNode>,
            crate::acquisition::Plan,
        ),
        Error,
    > {
        let presentation = spec.frame();
        if spec.is_inspection() {
            let parameters = input.parameters();
            let matrix = DMat3::from_cols(
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
        if crate::gpu::photograph::PhotographGraph::input_transform(&neutral)
            .is_some_and(|captured| captured != *program)
        {
            return Err(Error::Gpu("physical-view source is stale".into()));
        }
        if input.repairs != spec.repairs {
            return Err(Error::Gpu(
                "physical-view repairs differ from reconstructed originals".into(),
            ));
        }
        input.grade().unwrap().require_extent(spec.area)?;
        input.validate_repairs(limits, input.repairs.bound())?;
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

    pub fn request(
        context: &crate::gpu::evaluation::Context,
        root: Arc<crate::gpu::evaluation::ImageNode>,
    ) -> Result<crate::gpu::evaluation::Evaluation, Error> {
        let domain = root.domain();
        let bounds = domain.finite()?;
        context.request(
            root,
            crate::gpu::evaluation::Region {
                bounds,
                grid: domain.grid,
            },
            crate::gpu::evaluation::Budget {
                tile: [bounds.width, bounds.height],
                scratch_bytes: VIEW_SCRATCH_BYTES,
            },
        )
    }

    pub fn produced(
        source: Arc<AcquiredViewRaster>,
        definition: Arc<crate::gpu::evaluation::ImageNode>,
        plan: crate::acquisition::Plan,
        input: &ProcessingInput,
        value: crate::gpu::evaluation::Image<crate::gpu::evaluation::ImageLayout>,
    ) -> Result<Arc<Self>, Error> {
        let pixels: crate::gpu::evaluation::BufferImage = value.try_into()?;
        crate::gpu::evaluation::SuppliedImage::from_buffer(
            definition.clone(),
            definition.domain().finite()?,
            pixels.clone(),
        )?;
        Ok(Arc::new(Self {
            source,
            definition,
            plan,
            pixels,
            grade: input.grade_shared().cloned(),
            stages: None,
            submitted: Mutex::new(true),
        }))
    }

    #[cfg(test)]
    pub fn prepare(
        device: &wgpu::Device,
        source: Arc<AcquiredViewRaster>,
        input: &ProcessingInput,
        previous: Option<&Arc<Self>>,
    ) -> Result<Arc<Self>, Error> {
        let (root, plan) = Self::definition(
            &device.limits(),
            &source.spec,
            source.definition.clone(),
            input,
        )?;
        Self::prepare_calculation(device, source, input, root, plan, previous)
    }

    pub fn prepare_calculation(
        device: &wgpu::Device,
        source: Arc<AcquiredViewRaster>,
        input: &ProcessingInput,
        root: Arc<crate::gpu::evaluation::ImageNode>,
        plan: crate::acquisition::Plan,
        previous: Option<&Arc<Self>>,
    ) -> Result<Arc<Self>, Error> {
        if let Some(previous) = previous.filter(|previous| previous.supplies(&root, &source)) {
            return Ok(previous.clone());
        }
        if let Some(identity) = root.identity()
            && let Some(value) =
                source
                    .renderer
                    .evaluation
                    .materializations()
                    .find_ready::<crate::gpu::evaluation::ImageLayout>(identity, |layout| {
                        layout.domain == root.domain()
                            && Some(layout.region) == root.domain().bounds
                            && layout.format == crate::gpu::evaluation::ImageFormat::Rgba32Float
                            && layout.backing == crate::gpu::evaluation::ImageBacking::Buffer
                    })
        {
            return Self::produced(source, root, plan, input, value);
        }
        Ok(Arc::new(Self::record(device, source, input, root, plan)?))
    }

    pub(super) fn record(
        device: &wgpu::Device,
        source: Arc<AcquiredViewRaster>,
        input: &ProcessingInput,
        root: Arc<crate::gpu::evaluation::ImageNode>,
        plan: crate::acquisition::Plan,
    ) -> Result<Self, Error> {
        let mut raster = if source.spec.is_inspection() {
            crate::gpu::raster::Raster::frame(source.spec.frame())
        } else {
            crate::gpu::raster::Raster::planned(&plan)
        };
        if !source.spec.is_inspection() {
            raster.dimensions[2] = source.definition.domain().grid.coverage;
        }
        let supplied = source.supplied()?;
        let stages = crate::gpu::pixel_pipeline::Prepared::record_composition(
            device,
            raster,
            root.clone(),
            root.domain().finite()?,
            source.renderer.evaluation.clone(),
            VIEW_SCRATCH_BYTES,
            vec![supplied],
        )?;
        let pixels = crate::gpu::evaluation::BufferImage::recorded(
            &root,
            root.domain().finite()?,
            float_buffer(device, source.spec.frame().output)?,
        )?;
        Ok(Self {
            pixels,
            definition: root,
            grade: input.grade_shared().cloned(),
            plan,
            stages: Some(stages),
            source,
            submitted: Mutex::new(false),
        })
    }
}
