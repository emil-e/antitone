//! Photographs over already-sampled camera cells. The source owns their raster declaration.

use super::*;
use crate::photographic::stages::interpretation::{InputTransform, InspectionView};
use crate::{
    acquisition::raster::Raster,
    acquisition::{Plan, Request, Sampling},
    frame_raster::FrameRaster,
    geometry::Composition,
};

pub(crate) fn raster(source: &ProcessingSource, layout: [u32; 2]) -> Result<Raster, Error> {
    let Some(geometry) = source.geometry() else {
        return Ok(Raster::native([0, 0, layout[0], layout[1]].into()));
    };
    let region = geometry.region();
    let size = geometry.size();
    let composition = Composition::new(region, Default::default(), Default::default())?;
    let output = FrameRaster {
        output: size,
        ..FrameRaster::full(composition)?
    };
    let plan = Plan::new(Request {
        output,
        sampling: Sampling::CameraCells { region, size },
    })?;
    Ok(Raster::planned(&plan))
}

pub(crate) fn codes(source: &ProcessingSource, raster: Raster) -> Result<Value, Error> {
    if u64::from(raster.size()[0]) * u64::from(raster.size()[1]) != u64::from(source.pixel_count())
    {
        return Err(Error::InvalidParameter(
            "processing source layout differs from plan",
        ));
    }
    let domain = Domain {
        bounds: Some(CellRegion::new([0, 0], raster.size())),
        grid: Grid::from_raster(raster),
    };
    Ok(Value::from_operation(
        domain,
        None,
        ResidentCamera {
            domain,
            identity: Arc::downgrade(&source.identity),
            pixels: source.input.clone(),
            continuous: source.continuous,
        },
    ))
}

pub(crate) fn photograph(
    limits: &wgpu::Limits,
    camera: Value,
    source: &ProcessingSource,
    field: Option<Arc<GpuFlatField>>,
    program: InputTransform,
    grading: Grading,
    repairs: &[crate::dust_repair::Repair],
) -> Result<Value, Error> {
    let geometry = source.geometry();
    grading.require_geometry(geometry)?;
    if let Some(geometry) = geometry.filter(|geometry| geometry.is_pair()) {
        let area = grading.area();
        if geometry.regions().any(|region| {
            region.x < area.x
                || region.y < area.y
                || region.x >= area.x + area.width
                || region.y >= area.y + area.height
        }) {
            return Err(Error::Gpu(
                "composed input and source geometry regions differ".into(),
            ));
        }
    }
    if !repairs.is_empty() && geometry.is_none() {
        return Err(Error::Gpu(
            "repair rendering needs original source geometry".into(),
        ));
    }
    let grading = grading.for_grid(0).with_camera_coordinates(geometry);
    composition::admit_photograph(limits, program, &grading)?;
    validate_camera_range(program.matrix(), field.as_deref(), source.continuous)?;
    let mapping = Mapping::Resident(geometry);
    let corrected = camera_image(camera, field, mapping);
    composition::graded_source(corrected, program, mapping, &grading, repairs)
}

pub(crate) fn inspection(
    camera: Value,
    source: &ProcessingSource,
    field: Option<Arc<GpuFlatField>>,
    view: InspectionView,
) -> Result<Value, Error> {
    composition::inspection(
        camera,
        field,
        Mapping::Resident(source.geometry()),
        source.camera_matrix(),
        source.continuous,
        view,
    )
}
