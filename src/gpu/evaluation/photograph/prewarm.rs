use super::*;

impl Photograph {
    #[cfg(feature = "ui")]
    pub(crate) fn prewarm_resident(
        device: &wgpu::Device,
        renderer: &crate::renderer::GpuRenderer,
    ) -> Result<(), Error> {
        let source = ProcessingSource::new(device, &[[0u16; 4]], glam::DMat3::IDENTITY)?;
        for input in [
            ProcessingInput::from(ProcessingParameters::source()),
            ProcessingInput::new(
                &device.limits(),
                crate::processing::InputSettings::Digital,
                Default::default(),
                crate::adjustment_layers::AdjustmentLayerSnapshot::new(vec![])?,
                glam::DMat3::IDENTITY,
                [0, 0, 1, 1].into(),
            )?,
        ] {
            let image = Self::from_resident_with_stages(
                &device.limits(),
                &source,
                &input,
                crate::gpu::raster::Raster::native([0, 0, 1, 1].into()),
                &input.pixel_pipeline(),
            )?
            .processed;
            renderer.evaluation().prewarm_image(image)?;
        }
        Ok(())
    }

    #[cfg(feature = "editor")]
    pub(crate) fn prewarm_filtered(device: &wgpu::Device, context: &Context) -> Result<(), Error> {
        let domain = Domain {
            bounds: Some([0, 0, 1, 1].into()),
            grid: Grid {
                coverage: 1,
                ..Grid::axis_aligned([0.; 2], [1.; 2])
            },
            meaning: Meaning::Neutral,
        };
        Grading::prewarm(device, context, domain)?;
        context.prewarm_image(inspect(
            borrowed_input(domain),
            ProcessingParameters::source(),
        )?)
    }
}
