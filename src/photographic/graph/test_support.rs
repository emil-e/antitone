use super::*;
use crate::acquisition::camera::ProcessingSource;
use crate::photographic::parameters::ProcessingParameters;
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
use crate::photographic::graph::authoring::PhotographTaps;
use crate::photographic::test_inputs::AuthoredPhotograph;

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

    fn raster(source: &ProcessingSource) -> crate::acquisition::raster::Raster {
        if let Some(geometry) = source.geometry() {
            crate::acquisition::raster::Raster::region(geometry.region(), geometry.size())
        } else {
            crate::acquisition::raster::Raster::native([0, 0, source.pixel_count(), 1].into())
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
        self.encode_with_field(device, encoder, source, output, parameters, None);
    }

    pub fn encode_with_field(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &ProcessingSource,
        output: &wgpu::Buffer,
        parameters: ProcessingParameters,
        field: Option<Arc<GpuFlatField>>,
    ) {
        let raster = Self::raster(source);
        let domain = Domain {
            bounds: Some(crate::evaluation::CellRegion::new([0, 0], raster.size())),
            grid: Grid::from_raster(raster),
        };
        let camera = Value::from_operation(
            domain,
            None,
            ResidentCamera {
                domain,
                identity: Arc::downgrade(&source.identity),
                pixels: source.input.clone(),
                continuous: source.continuous,
            },
        );
        let matrix = source.camera_matrix();
        validate_camera_range(matrix, field.as_deref(), source.continuous).unwrap();
        let corrected = camera_image(camera, field, Mapping::Resident(source.geometry()));
        let image =
            composition::interpret_camera(corrected, Interpretation::Packed(parameters), matrix)
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
        input: &AuthoredPhotograph,
    ) -> Result<(), Error> {
        let image = PhotographTaps::from_resident_fixture(
            &device.limits(),
            source,
            input,
            Self::raster(source),
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
        image: Value,
    ) -> Result<(), Error> {
        self.context.require_device(device)?;
        let plan = self.context.request(
            image.clone(),
            Region {
                bounds: image.domain().finite().unwrap(),
                grid: image.domain().grid,
            },
            Tiling {
                tile: [
                    image.domain().finite().unwrap().width,
                    image.domain().finite().unwrap().height,
                ],
            },
        )?;
        self.context
            .record(plan, self.context.test_materializations(true), Vec::new())?
            .encode_into(device, encoder, &source.input, output)
    }
}
