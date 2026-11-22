use super::*;
use crate::gpu::evaluation;
#[cfg(any(feature = "ui", feature = "editor"))]
use crate::gpu::grading::Grading;
#[cfg(feature = "ui")]
use crate::gpu::interpretation::{InputTransform, InspectionView};
#[cfg(feature = "editor")]
use crate::gpu::parameters::ProcessingParameters;
#[cfg(feature = "ui")]
use crate::gpu::source::ProcessingSource;

/// Compile the common viewer paths before foreground command recording.
pub fn prewarm_viewer(
    context: &evaluation::Context,
    drt: &crate::gpu::drt::Targets,
) -> Result<(), Error> {
    context.prewarm_texture(borrowed_input(Domain {
        bounds: Some([0, 0, 1, 1].into()),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
    }))?;
    super::composition::prewarm_output(context, drt)?;
    #[cfg(feature = "editor")]
    PhotographGraph::prewarm_filtered(context, drt)?;
    #[cfg(feature = "ui")]
    PhotographGraph::prewarm_resident(context, drt)?;
    Ok(())
}

#[cfg(any(feature = "ui", feature = "editor"))]
impl PhotographGraph {
    #[cfg(feature = "ui")]
    pub(crate) fn prewarm_resident(
        context: &evaluation::Context,
        drt: &crate::gpu::drt::Targets,
    ) -> Result<(), Error> {
        let device = context.device();
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
        )?
        .processed;
        for image in [inspection, positive] {
            context.prewarm_image(image.clone())?;
            for target in [
                crate::rendering::OutputTarget::Srgb,
                crate::rendering::OutputTarget::DisplayP3,
            ] {
                context.prewarm_image(drt.get(target).apply(image.clone())?)?;
            }
        }
        Ok(())
    }

    #[cfg(feature = "editor")]
    pub(crate) fn prewarm_filtered(
        context: &evaluation::Context,
        drt: &crate::gpu::drt::Targets,
    ) -> Result<(), Error> {
        let domain = Domain {
            bounds: Some([0, 0, 1, 1].into()),
            grid: Grid {
                coverage: 1,
                ..Grid::axis_aligned([0.; 2], [1.; 2])
            },
        };
        let mut images = Grading::warmup_values(context.device(), domain)?;
        images.push(inspect(
            borrowed_input(domain),
            ProcessingParameters::source(),
        )?);
        for image in images {
            context.prewarm_image(image.clone())?;
            for target in [
                crate::rendering::OutputTarget::Srgb,
                crate::rendering::OutputTarget::DisplayP3,
            ] {
                context.prewarm_image(drt.get(target).apply(image.clone())?)?;
            }
        }
        Ok(())
    }
}
