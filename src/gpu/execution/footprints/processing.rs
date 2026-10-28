//! Captured filtered-AP1 grade/inspection preparation and reusable stage resources.
use super::*;
use crate::gpu::{GpuGrade, ProcessingInput, ProcessingParameters, ProcessingSignature};
use glam::{DMat3, Vec4};
use std::sync::Mutex;

pub(crate) struct Processed {
    pub source: Arc<Acquired>,
    pub grade: Option<Arc<GpuGrade>>,
    pub plan: crate::acquisition::Plan,
    pub stages: crate::gpu::pixel_pipeline::Prepared,
    pub pixels: wgpu::Buffer,
    pub view: ViewUniform,
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
            if input.pipeline().is_some()
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
            return Ok(Arc::new(Self {
                pixels: float_buffer(device, source.spec.frame().output)?,
                grade: None,
                plan: source.spec.plan.as_ref().clone(),
                stages: crate::gpu::pixel_pipeline::Prepared::with_programs(
                    device,
                    &Default::default(),
                    crate::gpu::raster::Raster::frame(source.spec.frame()),
                    None,
                    source.renderer.pull_programs.clone(),
                )?,
                view: ViewUniform {
                    dimensions: [
                        source.spec.frame().output[0],
                        source.spec.frame().output[1],
                        0,
                        0,
                    ],
                    source_transform: parameters.post,
                    ..bytemuck::Zeroable::zeroed()
                },
                source,
                submitted: Mutex::new(false),
                processing,
            }));
        }
        let pipeline = input
            .pipeline()
            .ok_or_else(|| Error::Gpu("positive view requires a processing pipeline".into()))?;
        if source.spec.program.as_ref() != Some(pipeline.input_program()) {
            return Err(Error::Gpu("physical-view source is stale".into()));
        }
        if input.repairs != source.spec.repairs {
            return Err(Error::Gpu(
                "physical-view repairs differ from reconstructed originals".into(),
            ));
        }
        pipeline.grade.require_extent(source.spec.area)?;
        pipeline.validate_repairs(device, input.repairs.bound())?;
        let stages = input.pixel_pipeline()?;
        let plan = source.spec.processing_plan(&stages)?;
        if let Some(previous) = previous
            .filter(|p| p.source.spec == source.spec && p.processing.same_photographic(&processing))
        {
            return Ok(previous.clone());
        }
        let view = ViewUniform::new(plan.processing_frame()?, pipeline.grade.parameters);
        let cached = previous
            .filter(|p| p.source.spec.same_source(&source.spec))
            .map(|p| &p.stages);
        let stages = crate::gpu::pixel_pipeline::Prepared::with_programs(
            device,
            plan.pipeline(),
            crate::gpu::raster::Raster::planned(&plan),
            cached,
            source.renderer.pull_programs.clone(),
        )?;
        Ok(Arc::new(Self {
            pixels: float_buffer(device, source.spec.frame().output)?,
            grade: Some(pipeline.grade.clone()),
            plan,
            stages,
            view,
            source,
            submitted: Mutex::new(false),
            processing,
        }))
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ViewUniform {
    pub(crate) dimensions: [u32; 4],
    origin: [f32; 4],
    dx: [f32; 4],
    dy: [f32; 4],
    gain: [f32; 4],
    pub(crate) source_transform: [Vec4; 3],
}
impl ViewUniform {
    pub(crate) fn new(frame: FrameRaster, parameters: ProcessingParameters) -> Self {
        let origin = frame.source_point([0.5, 0.5]).0;
        let x = frame.source_point([1.5, 0.5]).0;
        let y = frame.source_point([0.5, 1.5]).0;
        Self {
            dimensions: [frame.output[0], frame.output[1], 0, 0],
            origin: [origin[0] as f32, origin[1] as f32, 0., 0.],
            dx: [(x[0] - origin[0]) as f32, (x[1] - origin[1]) as f32, 0., 0.],
            dy: [(y[0] - origin[0]) as f32, (y[1] - origin[1]) as f32, 0., 0.],
            gain: [parameters.anchor.w.exp(), 0., 0., 0.],
            source_transform: parameters.post,
        }
    }
}
