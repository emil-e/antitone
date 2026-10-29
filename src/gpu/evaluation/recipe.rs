//! Explicit purpose assembly from one captured set of photographic components.
use super::*;
use crate::{
    gpu::{GpuFlatField, ProcessingInput, ProcessingSource},
    pixel_pipeline::PixelPipeline,
    rendering::OutputReference,
};

pub(crate) struct Recipe {
    input: ProcessingInput,
    stages: PixelPipeline,
}
impl Recipe {
    #[cfg(feature = "ui")]
    pub(crate) fn prewarm_resident(
        device: &wgpu::Device,
        renderer: &crate::renderer::GpuRenderer,
    ) -> Result<(), Error> {
        let source = ProcessingSource::new(device, &[[0u16; 4]], glam::DMat3::IDENTITY)?;
        let input = ProcessingInput::from(crate::gpu::ProcessingParameters::source());
        let image = Self::capture(&input)?.resident(
            device,
            renderer,
            &source,
            crate::gpu::raster::Raster::native([0, 0, 1, 1].into()),
        )?;
        renderer.evaluation().prewarm_image(image)
    }
    #[cfg(feature = "editor")]
    pub(crate) fn prewarm_filtered(device: &wgpu::Device, context: &Context) -> Result<(), Error> {
        let domain = Domain {
            bounds: [0, 0, 1, 1].into(),
            grid: Grid {
                coverage: 1,
                ..Grid::axis_aligned([0.; 2], [1.; 2])
            },
            meaning: Meaning::Neutral,
        };
        Grading::prewarm(device, context, domain)?;
        context.prewarm_image(Self::inspection(
            device,
            borrowed_input(domain),
            crate::gpu::ProcessingParameters::source(),
        )?)
    }
    pub fn capture(input: &ProcessingInput) -> Result<Self, Error> {
        Ok(Self::with_stages(input, input.pixel_pipeline()?))
    }
    pub fn with_stages(input: &ProcessingInput, stages: PixelPipeline) -> Self {
        Self {
            input: input.clone(),
            stages,
        }
    }
    pub fn stages(&self) -> &PixelPipeline {
        &self.stages
    }
    #[cfg(feature = "raw")]
    pub fn originals(
        &self,
        device: &wgpu::Device,
        renderer: &crate::renderer::GpuRenderer,
        source: (Arc<ImageNode>, Option<Arc<GpuFlatField>>, glam::DMat3),
        reference: OutputReference,
    ) -> Result<Arc<ImageNode>, Error> {
        let (image, field, matrix) = source;
        let photograph = Photograph::new(device, renderer, image, field, &self.input, matrix)?;
        self.reference(
            device,
            renderer,
            self.stages.compose(photograph.graded)?,
            reference,
        )
    }
    pub fn resident(
        &self,
        device: &wgpu::Device,
        renderer: &crate::renderer::GpuRenderer,
        source: &ProcessingSource,
        raster: crate::gpu::raster::Raster,
    ) -> Result<Arc<ImageNode>, Error> {
        let photograph = Photograph::from_resident(device, renderer, source, &self.input, raster)?;
        self.stages.compose(photograph.graded)
    }
    #[cfg(feature = "editor")]
    pub fn filtered(
        &self,
        device: &wgpu::Device,
        neutral: Arc<ImageNode>,
    ) -> Result<(Arc<ImageNode>, Grading), Error> {
        let grade = self
            .input
            .grade()
            .ok_or_else(|| Error::Gpu("positive view requires a processing pipeline".into()))?;
        let grading = Grading::new(device, grade, neutral.domain.grid.coverage);
        let image = self.stages.compose(grading.compose(neutral)?)?;
        Ok((image, grading))
    }
    #[cfg(feature = "editor")]
    pub fn inspection(
        device: &wgpu::Device,
        input: Arc<ImageNode>,
        parameters: crate::gpu::ProcessingParameters,
    ) -> Result<Arc<ImageNode>, Error> {
        use wgpu::util::DeviceExt;
        if input.domain.meaning != Meaning::Neutral {
            return Err(Error::InvalidParameter("inspection requires neutral AP1"));
        }
        let mut bytes = bytemuck::cast_slice(&parameters.post).to_vec();
        bytes.extend_from_slice(bytemuck::cast_slice(&[
            input.domain.grid.coverage,
            0u32,
            0,
            0,
        ]));
        let constants = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("source-inspection transform and coverage"),
            contents: &bytes,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        Ok(pointwise(
            input,
            Meaning::Neutral,
            Function {
                code: include_str!("inspection.wgsl").into(),
                symbol: "inspect_source",
                bindings: vec![(0, Shared::Buffer(constants).into())],
            },
        ))
    }
    pub fn reference(
        &self,
        device: &wgpu::Device,
        renderer: &crate::renderer::GpuRenderer,
        image: Arc<ImageNode>,
        reference: OutputReference,
    ) -> Result<Arc<ImageNode>, Error> {
        Reference::capture(device, renderer, reference, self.input.look())?.compose(image, renderer)
    }
}
