use super::*;
use crate::gpu::grading::Grading;
use crate::gpu::interpretation::{InputTransform, InspectionView};
use crate::gpu::parameters::ProcessingParameters;
use crate::gpu::photograph::PhotographGraph;
use crate::gpu::source::ProcessingSource;

impl PhotographGraph {
    #[cfg(feature = "ui")]
    pub(crate) fn prewarm_resident(
        device: &wgpu::Device,
        renderer: &crate::renderer::GpuRenderer,
    ) -> Result<(), Error> {
        let source = ProcessingSource::new(device, &[[0u16; 4]], glam::DMat3::IDENTITY, None)?;
        let camera = Self::resident_codes(
            &source,
            crate::gpu::raster::Raster::native([0, 0, 1, 1].into()),
        )?;
        let inspection = Self::resident_inspection(
            camera.clone(),
            &source,
            None,
            InspectionView::new(glam::DMat3::IDENTITY, 0.)?,
        )?;
        let positive = Self::resident_photograph(
            &device.limits(),
            camera,
            CameraInput {
                matrix: glam::DMat3::IDENTITY,
                field: None,
                continuous: source.continuous,
                mapping: Mapping::Resident(source.geometry()),
            },
            InputTransform::new(
                crate::processing::InputSettings::Digital,
                glam::DMat3::IDENTITY,
            )?,
            Grading::new(
                &device.limits(),
                Default::default(),
                crate::adjustment_layers::AdjustmentLayerSnapshot::new(vec![])?,
                [0, 0, 1, 1].into(),
            )?,
            Default::default(),
            &Default::default(),
        )?
        .processed;
        for image in [inspection, positive] {
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
            borrowed_input(Domain {
                meaning: Meaning::Inspection,
                ..domain
            }),
            ProcessingParameters::source(),
        )?)
    }
}
