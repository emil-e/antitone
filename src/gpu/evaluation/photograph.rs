//! Photographic image boundaries and their contracted camera-cell implementations.

use super::*;
use crate::gpu::{
    CameraResources, GpuFlatField, ProcessingInput, ProcessingParameters, ProcessingSource,
    SourceGeometry, flat::FieldGeometry,
};

mod assembly;
#[cfg(test)]
pub(crate) mod test_support;

use crate::gpu::photograph::Boundary;
use assembly::Assembly;

/// Captured camera, neutral and photographic image boundaries.
/// Canonical implementations fold exposure into inversion; a separately materialized neutral
/// prefix must independently fit f32. Resident camera inputs currently evaluate complete grids.
pub struct Photograph {
    pub camera: Arc<ImageNode>,
    pub unrepaired: Arc<ImageNode>,
    pub neutral: Arc<ImageNode>,
    pub roll: Arc<ImageNode>,
    pub graded: Arc<ImageNode>,
    pub(crate) processed: Arc<ImageNode>,
}

#[derive(Clone, Copy)]
enum Mapping {
    Originals,
    Resident(Option<SourceGeometry>),
}

impl Mapping {
    fn originals() -> Self {
        Self::Originals
    }

    fn geometry(self, image: PlannedImage) -> Result<Option<SourceGeometry>, Error> {
        match self {
            Self::Originals => {
                let r = image.region;
                SourceGeometry::original_tile([r.x, r.y, r.width, r.height], image.size()).map(Some)
            }
            Self::Resident(geometry) => Ok(geometry),
        }
    }
}

struct CameraInput {
    matrix: glam::DMat3,
    matrix_buffer: Option<wgpu::Buffer>,
    field: Option<Arc<GpuFlatField>>,
    continuous: bool,
    mapping: Mapping,
}

struct Captured {
    program: Option<crate::gpu::GpuInputProgram>,
    source_parameters: Option<ProcessingParameters>,
    matrix: crate::gpu::CameraMatrix,
    matrix_buffer: Option<wgpu::Buffer>,
    field: Option<Arc<GpuFlatField>>,
    mapping: Mapping,
    nodes: wgpu::Buffer,
    empty_layers: wgpu::Buffer,
    empty_context: wgpu::Buffer,
    source_limit: u64,
    code: Arc<str>,
}

impl Captured {
    fn new(
        device: &wgpu::Device,
        resources: &CameraResources,
        definition: CameraInput,
        program: Option<crate::gpu::GpuInputProgram>,
        source_parameters: Option<ProcessingParameters>,
    ) -> Result<Arc<Self>, Error> {
        let CameraInput {
            matrix,
            matrix_buffer,
            field,
            continuous,
            mapping,
        } = definition;
        let source_gain = if continuous {
            crate::processing::MAX_SOURCE_GAIN
        } else {
            1.
        };
        crate::gpu::CameraMatrix::new(matrix * source_gain)?;
        if let Some(field) = &field {
            field.validate_matrix(matrix, source_gain)?;
        }
        let mut matrix = crate::gpu::CameraMatrix::new(matrix)?;
        matrix.columns[0].w = if continuous { 1. } else { 0. };
        Ok(Arc::new(Self {
            program,
            source_parameters,
            matrix,
            matrix_buffer,
            field,
            mapping,
            nodes: resources.identity.clone(),
            empty_layers: resources.empty_adjustment_layers.clone(),
            empty_context: resources.empty_adjustment_layer_context.clone(),
            source_limit: device
                .limits()
                .max_storage_buffer_binding_size
                .min(device.limits().max_buffer_size)
                / 16,
            code: crate::gpu::CameraResources::module_shader().into(),
        }))
    }
}

struct Photographic {
    parameters: ProcessingParameters,
    grade: Option<Arc<crate::gpu::Grading>>,
    repairs: crate::gpu::repairs::Snapshot,
}

enum Output {
    Camera,
    Neutral,
    Roll(Arc<Photographic>),
    Graded(Arc<Photographic>),
}

impl Photograph {
    #[cfg(all(
        feature = "raw",
        feature = "project",
        any(feature = "editor", feature = "export")
    ))]
    pub(super) fn unrepaired_readout(
        device: &wgpu::Device,
        resources: &CameraResources,
        source: Arc<ImageNode>,
        field: Option<Arc<GpuFlatField>>,
        program: crate::gpu::GpuInputProgram,
        encoding: crate::processing::RollSettings,
    ) -> Result<Arc<ImageNode>, Error> {
        if source.domain.meaning != Meaning::CameraCodes
            || source.domain.grid != Grid::axis_aligned([0.; 2], [1.; 2])
        {
            return Err(Error::InvalidParameter(
                "photographic conversion requires original camera cells",
            ));
        }

        let parameters = ProcessingParameters::interpret(program.input, encoding, program.matrix)?;
        let capture = Captured::new(
            device,
            resources,
            CameraInput {
                matrix: program.matrix,
                matrix_buffer: None,
                field,
                continuous: true,
                mapping: Mapping::Originals,
            },
            None,
            Some(parameters),
        )?;

        let mut assembly = Assembly::captured(capture);
        assembly.encoded = encoding.exposure != 0.;

        assembly.prefix(source, Boundary::Codes, Boundary::Unrepaired)
    }

    #[cfg(feature = "editor")]
    pub(super) fn native_positive(
        camera: Arc<ImageNode>,
        program: crate::gpu::GpuInputProgram,
        encoding: crate::processing::RollSettings,
    ) -> Result<Arc<ImageNode>, Error> {
        Assembly::native(program, encoding).prefix(camera, Boundary::Camera, Boundary::Native)
    }

    #[cfg(feature = "editor")]
    pub(super) fn filtered(
        grading: &Grading,
        neutral: Arc<ImageNode>,
        stages: &crate::pixel_pipeline::PixelPipeline,
        output: Domain,
        offset: [u32; 2],
        scale: [f64; 2],
    ) -> Result<Arc<ImageNode>, Error> {
        Assembly::filtered(grading.clone()).filtered_image(neutral, stages, output, offset, scale)
    }

    #[cfg(feature = "editor")]
    pub(crate) fn camera_footprints(
        device: &wgpu::Device,
        resources: &CameraResources,
        source: Arc<ImageNode>,
        field: Option<Arc<GpuFlatField>>,
        origins: &[[u32; 2]],
    ) -> Result<Arc<ImageNode>, Error> {
        use wgpu::util::DeviceExt;

        let height = u32::try_from(origins.len())
            .ok()
            .and_then(|count| count.checked_mul(8))
            .filter(|height| *height > 0)
            .ok_or(Error::InvalidParameter("invalid camera footprint batch"))?;

        if source.domain.meaning != Meaning::CameraCodes
            || source.domain.bounds != [0, 0, 8, height].into()
            || source.domain.grid != Grid::axis_aligned([0.; 2], [1.; 2])
        {
            return Err(Error::InvalidParameter(
                "camera footprints require an original-cell index grid",
            ));
        }

        let captured = Captured::new(
            device,
            resources,
            CameraInput {
                matrix: glam::DMat3::IDENTITY,
                matrix_buffer: None,
                field,
                continuous: true,
                mapping: Mapping::Resident(None),
            },
            None,
            None,
        )?;
        let positions = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("captured original camera footprint origins"),
            contents: bytemuck::cast_slice(origins),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let geometry = if let Some(field) = &captured.field {
            let mapping = SourceGeometry::new(field.field().area(), [1, 1])?;
            FieldGeometry::new(field, mapping)
        } else {
            FieldGeometry::uncorrected(None)
        };
        let nodes = captured
            .field
            .as_ref()
            .map_or(&captured.nodes, |field| &field.nodes)
            .clone();
        let function = Function::new(
            format!(
                "{}\n{}",
                crate::gpu::CameraResources::kernel_shader(),
                include_str!("photograph/gathered.wgsl")
            ),
            "gathered_camera_light",
        )
        .with_shared(4, Shared::Buffer(nodes))
        .with_uniform(5, &geometry)
        .with_shared(18, Shared::Buffer(positions));
        let mut assembly = Assembly::captured(captured);
        assembly.camera_function = Some(function);

        assembly.prefix(source, Boundary::Codes, Boundary::Camera)
    }

    /// Corrected original camera light, excluding interpretation and recorded positive exposure.
    pub fn camera_light(
        device: &wgpu::Device,
        resources: &CameraResources,
        source: Arc<ImageNode>,
        field: Option<Arc<GpuFlatField>>,
    ) -> Result<Arc<ImageNode>, Error> {
        if source.domain.meaning != Meaning::CameraCodes
            || source.domain.grid != Grid::axis_aligned([0.; 2], [1.; 2])
        {
            return Err(Error::InvalidParameter(
                "camera-light conversion requires original camera cells",
            ));
        }

        let captured = Captured::new(
            device,
            resources,
            CameraInput {
                matrix: glam::DMat3::IDENTITY,
                matrix_buffer: None,
                field,
                continuous: true,
                mapping: Mapping::Originals,
            },
            None,
            None,
        )?;

        Assembly::captured(captured).prefix(source, Boundary::Codes, Boundary::Camera)
    }

    pub fn new(
        device: &wgpu::Device,
        resources: &CameraResources,
        source: Arc<ImageNode>,
        field: Option<Arc<GpuFlatField>>,
        input: &ProcessingInput,
        matrix: glam::DMat3,
    ) -> Result<Self, Error> {
        Self::new_with_stages(
            device,
            resources,
            source,
            field,
            input,
            matrix,
            &Default::default(),
        )
    }

    pub(crate) fn new_with_stages(
        device: &wgpu::Device,
        resources: &CameraResources,
        source: Arc<ImageNode>,
        field: Option<Arc<GpuFlatField>>,
        input: &ProcessingInput,
        matrix: glam::DMat3,
        stages: &crate::pixel_pipeline::PixelPipeline,
    ) -> Result<Self, Error> {
        if source.domain.meaning != Meaning::CameraCodes
            || source.domain.grid != Grid::axis_aligned([0.; 2], [1.; 2])
        {
            return Err(Error::InvalidParameter(
                "photographic conversion requires original camera cells",
            ));
        }
        input.repairs.require_area(source.domain.bounds)?;
        if let Some(grading) = input.grade() {
            grading.require_extent(source.domain.bounds)?;
        }
        Self::capture(
            device,
            resources,
            source,
            CameraInput {
                matrix,
                matrix_buffer: None,
                field,
                continuous: true,
                mapping: Mapping::originals(),
            },
            input,
            stages,
        )
    }

    pub(crate) fn from_resident_with_stages(
        device: &wgpu::Device,
        resources: &CameraResources,
        source: &ProcessingSource,
        input: &ProcessingInput,
        raster: crate::gpu::raster::Raster,
        stages: &crate::pixel_pipeline::PixelPipeline,
    ) -> Result<Self, Error> {
        input.require_source(source)?;
        if u64::from(raster.size()[0]) * u64::from(raster.size()[1])
            != u64::from(source.pixel_count())
        {
            return Err(Error::InvalidParameter(
                "processing source layout differs from plan",
            ));
        }
        if !input.repairs.0.is_empty() && source.geometry.is_none() {
            return Err(Error::Gpu(
                "repair rendering needs original source geometry".into(),
            ));
        }
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
        Self::capture(
            device,
            resources,
            camera,
            CameraInput {
                matrix: source.camera_to_ap1,
                matrix_buffer: Some(source.matrix.clone()),
                field: source.flat_field.clone(),
                continuous: source.continuous,
                mapping: Mapping::Resident(source.geometry),
            },
            input,
            stages,
        )
    }

    fn capture(
        device: &wgpu::Device,
        resources: &CameraResources,
        source: Arc<ImageNode>,
        definition: CameraInput,
        input: &ProcessingInput,
        stages: &crate::pixel_pipeline::PixelPipeline,
    ) -> Result<Self, Error> {
        let matrix = definition.matrix;
        let mapping = definition.mapping;
        if let Some(program) = input.source_program() {
            if program.matrix != matrix {
                return Err(Error::InvalidParameter(
                    "photographic conversion matrix differs from captured input",
                ));
            }
            input.validate_repairs(device, input.repairs.bound())?;
        }

        let captured = Captured::new(
            device,
            resources,
            definition,
            input.source_program().cloned(),
            (!input.parameters().graded()).then_some(input.parameters()),
        )?;
        let mut assembly = Assembly::captured(captured.clone());
        if input.parameters().graded() {
            assembly.repairs = input.repairs.clone();
        }

        if input.parameters().graded() {
            assembly.photographic = Some(Arc::new(Photographic {
                parameters: input.parameters(),
                grade: input.grade_shared().cloned(),
                repairs: input.repairs.clone(),
            }));
            let grading = input
                .grade()
                .ok_or(Error::InvalidParameter(
                    "photographic input requires captured interpretation and grading",
                ))?
                .for_grid(0);
            assembly.grading = Some(match mapping {
                Mapping::Resident(geometry) => grading.with_camera_coordinates(geometry),
                Mapping::Originals => grading,
            });
        }

        assembly.photograph(source, stages)
    }
}

/// Adapter-captured neutral conversion, with no downstream photographic grade ownership.
#[cfg(feature = "editor")]
pub(crate) struct Neutral {
    pub parameters: ProcessingParameters,
    pub matrix: glam::DMat3,
    pub field: Option<Arc<GpuFlatField>>,
    pub repairs: crate::gpu::repairs::Snapshot,
}

#[cfg(feature = "editor")]
impl Neutral {
    pub fn originals(
        &self,
        device: &wgpu::Device,
        resources: &CameraResources,
        source: Arc<ImageNode>,
    ) -> Result<Arc<ImageNode>, Error> {
        if source.domain.meaning != Meaning::CameraCodes
            || source.domain.grid != Grid::axis_aligned([0.; 2], [1.; 2])
        {
            return Err(Error::InvalidParameter(
                "neutral conversion requires original camera cells",
            ));
        }
        self.repairs.require_area(source.domain.bounds)?;
        self.compose(
            device,
            resources,
            source,
            CameraInput {
                matrix: self.matrix,
                matrix_buffer: None,
                field: self.field.clone(),
                continuous: true,
                mapping: Mapping::Originals,
            },
        )
    }

    pub fn resident(
        &self,
        device: &wgpu::Device,
        resources: &CameraResources,
        source: &ProcessingSource,
        raster: crate::gpu::raster::Raster,
    ) -> Result<Arc<ImageNode>, Error> {
        if u64::from(raster.size()[0]) * u64::from(raster.size()[1])
            != u64::from(source.pixel_count())
        {
            return Err(Error::InvalidParameter(
                "neutral source layout differs from its grid",
            ));
        }
        if source.camera_to_ap1 != self.matrix {
            return Err(Error::InvalidParameter(
                "neutral source matrix differs from captured input",
            ));
        }
        let domain = Domain {
            bounds: [0, 0, raster.size()[0], raster.size()[1]].into(),
            grid: Grid::from_raster(raster),
            meaning: Meaning::CameraCodes,
        };
        let image = ImageNode::new(
            domain,
            None,
            ResidentCamera {
                domain,
                bytes: source.byte_len(),
            },
        );
        self.compose(
            device,
            resources,
            image,
            CameraInput {
                matrix: self.matrix,
                matrix_buffer: Some(source.matrix.clone()),
                field: self.field.clone(),
                continuous: source.continuous,
                mapping: Mapping::Resident(source.geometry),
            },
        )
    }

    fn compose(
        &self,
        device: &wgpu::Device,
        resources: &CameraResources,
        source: Arc<ImageNode>,
        definition: CameraInput,
    ) -> Result<Arc<ImageNode>, Error> {
        let captured = Captured::new(device, resources, definition, None, Some(self.parameters))?;
        let mut assembly = Assembly::captured(captured);
        assembly.repairs = self.repairs.clone();

        assembly.prefix(source, Boundary::Codes, Boundary::Neutral)
    }
}

struct ResidentCamera {
    domain: Domain,
    bytes: u64,
}

impl Operation for ResidentCamera {
    fn input(
        &self,
        _: crate::geometry::SourceRegion,
    ) -> Result<Option<crate::geometry::SourceRegion>, Error> {
        Ok(None)
    }

    fn lower(
        &self,
        fragment: &mut Fragment,
        _: Option<PlannedImage>,
        output: PlannedImage,
    ) -> Result<(), Error> {
        if output.region != self.domain.bounds {
            return Err(Error::InvalidParameter(
                "resident camera evaluation requires the complete grid",
            ));
        }
        fragment.resources[output.resource.0] = Storage::Input {
            slot: 0,
            bytes: self.bytes,
            writable: false,
        };
        Ok(())
    }
}

struct Camera {
    captured: Arc<Captured>,
    output: Output,
}

impl Camera {
    fn parameters(&self) -> Result<ProcessingParameters, Error> {
        match &self.output {
            Output::Camera => Ok(ProcessingParameters::source()),
            Output::Neutral => match &self.captured.program {
                Some(program) => program.neutral_parameters(),
                None => self
                    .captured
                    .source_parameters
                    .ok_or(Error::InvalidParameter(
                        "neutral prefix requires captured input interpretation",
                    )),
            },
            Output::Roll(photo) | Output::Graded(photo) => Ok(photo.parameters),
        }
    }
}

impl Operation for Camera {
    fn input(
        &self,
        region: crate::geometry::SourceRegion,
    ) -> Result<Option<crate::geometry::SourceRegion>, Error> {
        Ok(Some(region))
    }

    fn lower(
        &self,
        f: &mut Fragment,
        input: Option<PlannedImage>,
        output: PlannedImage,
    ) -> Result<(), Error> {
        let input = input.unwrap();
        let capture = &self.captured;
        if u64::from(input.cells()) > capture.source_limit {
            return Err(Error::InvalidParameter(
                "processing unit exceeds source work budget",
            ));
        }
        let geometry = capture.mapping.geometry(input)?;
        let field = match &capture.field {
            Some(field) => FieldGeometry::new(
                field,
                geometry.ok_or(Error::InvalidParameter(
                    "corrected source requires geometry",
                ))?,
            ),
            None => FieldGeometry::uncorrected(geometry),
        };
        let (layers, context) = if let Output::Graded(photo) = &self.output {
            photo
                .grade
                .as_ref()
                .map(|grade| (&grade.buffer, &grade.context))
                .unwrap_or((&capture.empty_layers, &capture.empty_context))
        } else {
            (&capture.empty_layers, &capture.empty_context)
        };
        let packed = self.parameters()?;
        let parameters = f.uniform(&packed);
        let matrix = if matches!(self.output, Output::Camera) {
            let mut matrix = crate::gpu::CameraMatrix::new(glam::DMat3::IDENTITY)?;
            matrix.columns[0].w = capture.matrix.columns[0].w;
            f.uniform(&matrix)
        } else {
            capture.matrix_buffer.as_ref().map_or_else(
                || f.uniform(&capture.matrix),
                |buffer| Binding::Shared(Shared::Buffer(buffer.clone())),
            )
        };
        let coordinates = f.uniform(&field);
        f.compute(
            capture.code.clone(),
            "main",
            vec![
                (0, Binding::Read(input.resource)),
                (1, Binding::Write(output.resource)),
                (2, parameters),
                (3, matrix),
                (
                    4,
                    Binding::Shared(Shared::Buffer(
                        capture
                            .field
                            .as_ref()
                            .map_or(&capture.nodes, |field| &field.nodes)
                            .clone(),
                    )),
                ),
                (5, coordinates),
                (6, Binding::Shared(Shared::Buffer(layers.clone()))),
                (7, Binding::Shared(Shared::Buffer(context.clone()))),
            ],
            output.cells(),
        );
        {
            let photo = match &self.output {
                Output::Camera | Output::Neutral => return Ok(()),
                Output::Roll(photo) | Output::Graded(photo) => photo,
            };
            if !packed.graded() || photo.repairs.0.is_empty() {
                return Ok(());
            }
            let Some((region, geometry)) = repair_geometry(geometry) else {
                return Ok(());
            };
            let grade = photo.grade.as_ref().ok_or(Error::InvalidParameter(
                "repair grading requires a photographic grade",
            ))?;
            let parameters = f.uniform(&grade.parameters);
            let geometry = f.uniform(&FieldGeometry::uncorrected(Some(geometry)));
            let tile = f.uniform(&[
                region.x,
                region.y,
                region.width,
                region.height,
                0u32,
                0,
                0,
                0,
            ]);
            for patch in &photo.repairs.0 {
                let pixels = patch.fragment_pixels(region);
                if pixels.is_empty() {
                    continue;
                }
                let count = pixels.len() as u32;
                let data = f.resource(Storage::Data(bytemuck::cast_slice(&pixels).into()));
                f.compute(
                    capture.code.clone(),
                    "repair_original",
                    vec![
                        (1, Binding::Write(output.resource)),
                        (2, parameters.clone()),
                        (5, geometry.clone()),
                        (6, Binding::Shared(Shared::Buffer(layers.clone()))),
                        (7, Binding::Shared(Shared::Buffer(context.clone()))),
                        (12, Binding::Read(data)),
                        (13, tile.clone()),
                    ],
                    count,
                );
            }
        }
        Ok(())
    }
}

fn repair_geometry(
    geometry: Option<SourceGeometry>,
) -> Option<(crate::geometry::SourceRegion, SourceGeometry)> {
    let geometry = geometry?;
    let region = geometry.region();
    (geometry.size() == [region.width, region.height]).then_some((region, geometry))
}

struct Repairs {
    snapshot: crate::gpu::repairs::Snapshot,
    code: Arc<str>,
    mapping: Mapping,
}

impl Operation for Repairs {
    fn input(
        &self,
        region: crate::geometry::SourceRegion,
    ) -> Result<Option<crate::geometry::SourceRegion>, Error> {
        Ok(Some(region))
    }

    fn in_place(&self) -> bool {
        true
    }

    fn lower(
        &self,
        f: &mut Fragment,
        _: Option<PlannedImage>,
        output: PlannedImage,
    ) -> Result<(), Error> {
        let Some((region, geometry)) = repair_geometry(self.mapping.geometry(output)?) else {
            return Ok(());
        };
        let geometry = f.uniform(&FieldGeometry::uncorrected(Some(geometry)));
        let tile = f.uniform(&[
            region.x,
            region.y,
            region.width,
            region.height,
            0u32,
            0,
            0,
            0,
        ]);
        for patch in &self.snapshot.0 {
            let pixels = patch.fragment_pixels(region);
            if pixels.is_empty() {
                continue;
            }
            let count = pixels.len() as u32;
            let data = f.resource(Storage::Data(bytemuck::cast_slice(&pixels).into()));
            f.compute(
                self.code.clone(),
                "repair_neutral",
                vec![
                    (1, Binding::Write(output.resource)),
                    (5, geometry.clone()),
                    (12, Binding::Read(data)),
                    (13, tile.clone()),
                ],
                count,
            );
        }
        Ok(())
    }
}
