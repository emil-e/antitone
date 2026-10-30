//! Captured filtered-AP1 grade/inspection preparation and reusable stage resources.

use super::*;
use crate::gpu::{Grading, ProcessingInput, ProcessingParameters, ProcessingSignature};
use glam::DMat3;
use std::sync::Mutex;

// Required working storage is admitted independently of generated-value retention.
// Large presentations can keep their grid without an implicit coarse-quality fallback.
const VIEW_SCRATCH_BYTES: u64 = 1280 << 20;

pub(crate) struct Processed {
    pub source: Arc<Acquired>,
    pub grade: Option<Arc<Grading>>,
    pub grading: Option<crate::gpu::evaluation::Grading>,
    pub plan: crate::acquisition::Plan,
    pub stages: crate::gpu::pixel_pipeline::Prepared,
    pub pixels: wgpu::Buffer,
    pub submitted: Mutex<bool>,
    processing: ProcessingSignature,
}

impl Processed {
    pub fn prepare(
        device: &wgpu::Device,
        source: Arc<Acquired>,
        input: &ProcessingInput,
        previous: Option<&Arc<Self>>,
    ) -> Result<Arc<Self>, Error> {
        let processing = input.signature();
        if source.spec.inspection {
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
            if let Some(previous) =
                previous.filter(|p| p.source.spec == source.spec && p.processing == processing)
            {
                return Ok(previous.clone());
            }
            let raster = crate::gpu::raster::Raster::frame(source.spec.frame());
            let domain = crate::gpu::evaluation::Domain {
                bounds: [0, 0, raster.size()[0], raster.size()[1]].into(),
                grid: crate::gpu::evaluation::Grid::from_raster(raster),
                meaning: crate::gpu::evaluation::Meaning::Neutral,
            };
            let root = crate::gpu::evaluation::Recipe::inspection(
                device,
                crate::gpu::evaluation::borrowed_input(domain),
                parameters,
            )?;
            let stages = crate::gpu::pixel_pipeline::Prepared::from_composition(
                device,
                raster,
                root,
                None,
                source.renderer.evaluation.clone(),
            )?;
            return Ok(Arc::new(Self {
                pixels: float_buffer(device, source.spec.frame().output)?,
                grade: None,
                grading: None,
                plan: source.spec.plan.as_ref().clone(),
                stages,
                source,
                submitted: Mutex::new(false),
                processing,
            }));
        }
        let program = input.source_program().ok_or_else(|| {
            Error::Gpu("positive view requires captured source interpretation".into())
        })?;
        if source.spec.program.as_ref() != Some(program) {
            return Err(Error::Gpu("physical-view source is stale".into()));
        }
        if input.repairs != source.spec.repairs {
            return Err(Error::Gpu(
                "physical-view repairs differ from reconstructed originals".into(),
            ));
        }
        input.grade().unwrap().require_extent(source.spec.area)?;
        input.validate_repairs(device, input.repairs.bound())?;
        let recipe = crate::gpu::evaluation::Recipe::capture(input)?;
        let plan = source.spec.processing_plan(recipe.stages())?;
        if let Some(previous) = previous
            .filter(|p| p.source.spec == source.spec && p.processing.same_photographic(&processing))
        {
            return Ok(previous.clone());
        }
        let cached = previous
            .filter(|p| p.source.spec.same_source(&source.spec))
            .map(|p| &p.stages);
        let mut raster = crate::gpu::raster::Raster::planned(&plan);
        // Acquired neutral pixels carry coverage, including supplied camera cells whose
        // original raster carries only inversion-floor flags before cover_proxy.
        raster.dimensions[2] = 8;
        let domain = crate::gpu::evaluation::Domain {
            bounds: [0, 0, raster.size()[0], raster.size()[1]].into(),
            grid: crate::gpu::evaluation::Grid::from_raster(raster),
            meaning: crate::gpu::evaluation::Meaning::Neutral,
        };
        let presentation = source.spec.frame();
        let output = crate::gpu::evaluation::Domain {
            bounds: [0, 0, presentation.output[0], presentation.output[1]].into(),
            grid: crate::gpu::evaluation::Grid::from_raster(crate::gpu::raster::Raster::frame(
                presentation,
            )),
            meaning: crate::gpu::evaluation::Meaning::Graded,
        };
        let scale = std::array::from_fn(|i| {
            f64::from(plan.output().output[i]) / f64::from(presentation.output[i])
        });
        let (root, grading) = recipe.filtered(
            crate::gpu::evaluation::borrowed_input(domain),
            output,
            plan.output_offset().expect("frame-grid output"),
            scale,
        )?;
        let stages = crate::gpu::pixel_pipeline::Prepared::record_composition(
            device,
            raster,
            root,
            cached,
            source.renderer.evaluation.clone(),
            VIEW_SCRATCH_BYTES,
        )?;
        Ok(Arc::new(Self {
            pixels: float_buffer(device, source.spec.frame().output)?,
            grade: input.grade_shared().cloned(),
            grading: Some(grading),
            plan,
            stages,
            source,
            submitted: Mutex::new(false),
            processing,
        }))
    }
}
