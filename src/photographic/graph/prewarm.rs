use super::*;
#[cfg(feature = "ui")]
use crate::acquisition::camera::ProcessingSource;
use crate::evaluation;
#[cfg(feature = "editor")]
use crate::photographic::parameters::ProcessingParameters;
#[cfg(any(feature = "ui", feature = "editor"))]
use crate::photographic::stages::grading::Grading;
#[cfg(feature = "ui")]
use crate::photographic::stages::interpretation::{InputTransform, InspectionView};

/// Compile the common viewer paths before foreground command recording.
pub fn prewarm_viewer(
    context: &evaluation::Context,
    drt: &crate::photographic::stages::display_rendering::Targets,
) -> Result<(), Error> {
    context.prewarm_texture(borrowed_input(Domain {
        bounds: Some([0, 0, 1, 1].into()),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
    }))?;
    super::composition::prewarm_output(context, drt)?;
    #[cfg(feature = "editor")]
    prewarm_filtered(context, drt)?;
    #[cfg(feature = "ui")]
    prewarm_resident(context, drt)?;
    Ok(())
}

#[cfg(feature = "ui")]
pub(crate) fn prewarm_resident(
    context: &evaluation::Context,
    drt: &crate::photographic::stages::display_rendering::Targets,
) -> Result<(), Error> {
    let device = context.device();
    let source = ProcessingSource::new(device, &[[0u16; 4]], glam::DMat3::IDENTITY, None)?;
    let camera = resident::codes(
        &source,
        crate::acquisition::raster::Raster::native([0, 0, 1, 1].into()),
    )?;
    let inspection = resident::inspection(
        camera.clone(),
        &source,
        None,
        InspectionView::new(glam::DMat3::IDENTITY, 0.)?,
    )?;
    let positive = resident::photograph(
        &device.limits(),
        camera,
        &source,
        None,
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
        &[],
    )?;
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
    drt: &crate::photographic::stages::display_rendering::Targets,
) -> Result<(), Error> {
    use crate::photographic::stages::interpretation::inspect_filtered;

    let domain = Domain {
        bounds: Some([0, 0, 1, 1].into()),
        grid: Grid {
            coverage: 1,
            ..Grid::axis_aligned([0.; 2], [1.; 2])
        },
    };
    let mut images = Grading::warmup_values(context.device(), domain)?;
    images.push(inspect_filtered(
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
