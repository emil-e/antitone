use super::*;

pub(crate) struct CameraFixture {
    resources: CameraResources,
    context: Context,
}

impl CameraFixture {
    pub async fn new(device: &wgpu::Device) -> Result<Self, Error> {
        Ok(Self {
            resources: CameraResources::new(device)?,
            context: Context::new(device, Default::default()),
        })
    }

    #[cfg(feature = "render")]
    pub fn with_context(resources: CameraResources, context: Context) -> Self {
        Self { resources, context }
    }

    fn raster(source: &ProcessingSource) -> crate::gpu::raster::Raster {
        if let Some(geometry) = source.geometry {
            crate::gpu::raster::Raster::region(geometry.region(), geometry.size())
        } else {
            crate::gpu::raster::Raster::native([0, 0, source.pixel_count(), 1].into())
        }
    }

    /// Exercise the actual fused camera component with packed primitive-test constants.
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
            bounds: [0, 0, raster.size()[0], raster.size()[1]].into(),
            grid: Grid::from_raster(raster),
            meaning: Meaning::CameraCodes,
        };
        let camera = ImageNode::new(
            domain,
            None,
            ResidentCamera {
                domain,
                bytes: source.byte_len(),
            },
        );
        let captured = Captured::new(
            device,
            &self.resources,
            CameraInput {
                matrix: source.camera_to_ap1,
                matrix_buffer: Some(source.matrix.clone()),
                field: source.flat_field.clone(),
                continuous: source.continuous,
                mapping: Mapping::Resident(source.geometry),
            },
            None,
            Some(parameters),
        )
        .unwrap();
        let image = ImageNode::new(
            Domain {
                meaning: Meaning::Neutral,
                ..domain
            },
            Some(camera),
            Camera {
                captured,
                output: Output::Neutral,
            },
        );
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
        let stages = input.pixel_pipeline()?;
        let image = Photograph::from_resident_with_stages(
            device,
            &self.resources,
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
                bounds: image.domain.bounds,
                grid: image.domain.grid,
            },
            Budget {
                tile: [image.domain.bounds.width, image.domain.bounds.height],
                scratch_bytes: device.limits().max_buffer_size.saturating_mul(4),
            },
        )?;
        self.context
            .record(plan, vec![])?
            .encode_into(device, encoder, &source.input, output)
    }
}
