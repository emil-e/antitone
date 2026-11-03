use super::*;
use crate::gpu::parameters::ProcessingParameters;
use crate::gpu::source::ProcessingSource;
use crate::{
    adjustment_layers::{
        AdjustmentLayer, AdjustmentLayerCoverage, AdjustmentLayerSettings, AdjustmentLayerSnapshot,
    },
    geometry::SourcePoint,
    processing::RollSettings,
};
use glam::{DMat3, DVec3};

pub fn matrix() -> DMat3 {
    DMat3::from_cols(
        DVec3::new(1.07, 0.03, -0.08),
        DVec3::new(0.01, 0.95, 0.02),
        DVec3::new(-0.08, 0.02, 1.06),
    )
}

pub fn settings(area: CellRegion) -> (RollSettings, AdjustmentLayerSnapshot) {
    let roll = RollSettings {
        exposure: 0.15,
        ..Default::default()
    };
    let mut local = AdjustmentLayer::global(AdjustmentLayerSettings {
        exposure: -0.2,
        contrast: 1.3,
        saturation: 0.85,
        ..Default::default()
    });
    local.mask = AdjustmentLayerCoverage::Linear {
        start: SourcePoint([f64::from(area.x), f64::from(area.y)]),
        end: SourcePoint([f64::from(area.right()), f64::from(area.bottom())]),
    };
    let snapshot = AdjustmentLayerSnapshot::new(vec![
        AdjustmentLayer::global(AdjustmentLayerSettings {
            contrast: 1.15,
            saturation: 1.1,
            ..Default::default()
        }),
        local,
    ])
    .unwrap();
    (roll, snapshot)
}
use crate::gpu::input::ProcessingInput;
use crate::gpu::photograph::PhotographGraph;

pub(crate) struct CameraFixture {
    context: Context,
}

impl CameraFixture {
    pub async fn new(device: &wgpu::Device) -> Result<Self, Error> {
        Ok(Self {
            context: Context::new(device, Default::default()),
        })
    }

    #[cfg(feature = "render")]
    pub fn with_context(context: Context) -> Self {
        Self { context }
    }

    fn raster(source: &ProcessingSource) -> crate::gpu::raster::Raster {
        if let Some(geometry) = source.geometry {
            crate::gpu::raster::Raster::region(geometry.region(), geometry.size())
        } else {
            crate::gpu::raster::Raster::native([0, 0, source.pixel_count(), 1].into())
        }
    }

    /// Exercise the photographic operations with packed primitive-test constants.
    /// Whole-photograph tests use captured interpretation/grading through encode_grade instead.
    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &ProcessingSource,
        output: &wgpu::Buffer,
        parameters: ProcessingParameters,
    ) {
        let raster = Self::raster(source);
        let domain = Domain {
            bounds: Some(crate::gpu::evaluation::CellRegion::new(
                [0, 0],
                raster.size(),
            )),
            grid: Grid::from_raster(raster),
            meaning: Meaning::CameraCodes,
        };
        let camera = ImageNode::new(
            domain,
            None,
            ResidentCamera {
                domain,
                pixels: source.input.clone(),
                continuous: source.continuous,
            },
        );
        let captured = Captured::new(
            CameraInput {
                matrix: source.camera_to_ap1,
                field: source.flat_field.clone(),
                continuous: source.continuous,
                mapping: Mapping::Resident(source.geometry),
            },
            Interpretation::Packed(parameters),
        )
        .unwrap();
        let image = Assembly::captured(captured)
            .prefix(camera, Boundary::Codes, Boundary::Neutral)
            .unwrap();
        self.record(device, encoder, source, output, image).unwrap();
    }

    /// Whole-photograph recording uses application assembly and common evaluation.
    pub fn encode_grade(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &ProcessingSource,
        output: &wgpu::Buffer,
        input: &ProcessingInput,
    ) -> Result<(), Error> {
        let stages = input.pixel_pipeline();
        let image = PhotographGraph::from_resident_with_stages(
            &device.limits(),
            source,
            input,
            Self::raster(source),
            &stages,
        )?
        .processed;
        self.record(device, encoder, source, output, image)
    }

    fn record(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &ProcessingSource,
        output: &wgpu::Buffer,
        image: Arc<ImageNode>,
    ) -> Result<(), Error> {
        self.context.require_device(device)?;
        let plan = self.context.request(
            image.clone(),
            Region {
                bounds: image.domain().finite().unwrap(),
                grid: image.domain().grid,
            },
            Budget {
                tile: [
                    image.domain().finite().unwrap().width,
                    image.domain().finite().unwrap().height,
                ],
                scratch_bytes: device.limits().max_buffer_size.saturating_mul(4),
            },
        )?;
        self.context
            .record(plan, self.context.test_materializations(true), Vec::new())?
            .encode_into(device, encoder, &source.input, output)
    }
}
