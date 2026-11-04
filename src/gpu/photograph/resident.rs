//! Photographs over already-sampled camera cells. The source owns their raster declaration.

use super::*;
use crate::{
    acquisition::{Plan, Request, Sampling},
    frame_raster::FrameRaster,
    geometry::Composition,
    gpu::{
        interpretation::{InputTransform, InspectionView},
        raster::Raster,
    },
    pixel_pipeline::PixelPipeline,
};

impl PhotographGraph {
    pub(crate) fn resident_raster(
        source: &ProcessingSource,
        stages: &PixelPipeline,
        layout: [u32; 2],
    ) -> Result<Raster, Error> {
        let Some(geometry) = source.geometry() else {
            if !stages.is_empty() {
                return Err(Error::Gpu(
                    "pixel processing requires declared raster geometry".into(),
                ));
            }
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
            stages,
            sampling: Sampling::CameraCells { region, size },
        })?;
        Ok(Raster::planned(&plan))
    }

    pub(crate) fn resident_codes(
        source: &ProcessingSource,
        raster: Raster,
    ) -> Result<Arc<ImageNode>, Error> {
        if u64::from(raster.size()[0]) * u64::from(raster.size()[1])
            != u64::from(source.pixel_count())
        {
            return Err(Error::InvalidParameter(
                "processing source layout differs from plan",
            ));
        }
        let domain = Domain {
            bounds: Some(CellRegion::new([0, 0], raster.size())),
            grid: Grid::from_raster(raster),
            meaning: Meaning::CameraCodes,
        };
        Ok(ImageNode::new(
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

    pub(crate) fn resident_photograph(
        limits: &wgpu::Limits,
        camera: Arc<ImageNode>,
        definition: CameraInput,
        program: InputTransform,
        grading: Grading,
        repairs: crate::gpu::repairs::Snapshot,
        stages: &PixelPipeline,
    ) -> Result<Self, Error> {
        let Mapping::Resident(geometry) = definition.mapping else {
            return Err(Error::InvalidParameter(
                "resident photograph requires camera-cell mapping",
            ));
        };
        grading.require_geometry(geometry)?;
        if let Some(geometry) = geometry.filter(|geometry| geometry.is_pair()) {
            let area = grading.area;
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
        if !repairs.0.is_empty() && geometry.is_none() {
            return Err(Error::Gpu(
                "repair rendering needs original source geometry".into(),
            ));
        }
        Assembly::photographic(
            limits,
            definition,
            program,
            grading.for_grid(0).with_camera_coordinates(geometry),
            repairs,
        )?
        .photograph(camera, stages)
    }

    pub(crate) fn resident_inspection(
        camera: Arc<ImageNode>,
        source: &ProcessingSource,
        field: Option<Arc<GpuFlatField>>,
        view: InspectionView,
    ) -> Result<Arc<ImageNode>, Error> {
        CameraGraph::new(
            camera,
            CameraInput {
                matrix: source.camera_matrix(),
                field,
                continuous: source.continuous,
                mapping: Mapping::Resident(source.geometry()),
            },
        )?
        .inspection(view)
    }
}
