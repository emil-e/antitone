//! Photographic image boundaries and their contracted camera-cell implementations.

use super::*;
use crate::gpu::{
    GpuFlatField, ProcessingInput, ProcessingParameters, ProcessingSource, SourceGeometry,
    flat::FieldGeometry,
};

/// Captured camera, neutral and photographic image boundaries.
/// Canonical implementations fold exposure into inversion; a separately materialized neutral
/// prefix must independently fit f32. Resident camera inputs currently evaluate complete grids.
pub struct Photograph {
    pub camera: Arc<ImageNode>,
    pub unrepaired: Arc<ImageNode>,
    pub neutral: Arc<ImageNode>,
    pub roll: Arc<ImageNode>,
    pub graded: Arc<ImageNode>,
}

#[derive(Clone, Copy)]
enum Mapping {
    #[cfg(feature = "raw")]
    Originals,
    Resident(Option<SourceGeometry>),
}

impl Mapping {
    fn geometry(self, image: PlannedImage) -> Result<Option<SourceGeometry>, Error> {
        match self {
            #[cfg(feature = "raw")]
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
        renderer: &crate::renderer::GpuRenderer,
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
            nodes: renderer.kernel.identity.clone(),
            empty_layers: renderer.kernel.empty_adjustment_layers.clone(),
            empty_context: renderer.kernel.empty_adjustment_layer_context.clone(),
            source_limit: device
                .limits()
                .max_storage_buffer_binding_size
                .min(device.limits().max_buffer_size)
                / 16,
            code: crate::gpu::ProcessingKernel::module_shader().into(),
        }))
    }
}

struct Photographic {
    parameters: ProcessingParameters,
    grade: Option<Arc<crate::gpu::GpuGrade>>,
    #[cfg(feature = "raw")]
    repairs: crate::gpu::repairs::Snapshot,
}

enum Output {
    Neutral,
    Roll(Arc<Photographic>),
    Graded(Arc<Photographic>),
}

impl Photograph {
    #[cfg(feature = "raw")]
    pub fn new(
        device: &wgpu::Device,
        renderer: &crate::renderer::GpuRenderer,
        source: Arc<ImageNode>,
        field: Option<Arc<GpuFlatField>>,
        input: &ProcessingInput,
        matrix: glam::DMat3,
    ) -> Result<Self, Error> {
        if source.domain.meaning != Meaning::CameraCodes
            || source.domain.grid != Grid::axis_aligned([0.; 2], [1.; 2])
        {
            return Err(Error::InvalidParameter(
                "photographic conversion requires original camera cells",
            ));
        }
        input.repairs.require_area(source.domain.bounds)?;
        if let Some(pipeline) = input.pipeline() {
            pipeline.grade.require_extent(source.domain.bounds)?;
        }
        Self::capture(
            device,
            renderer,
            source,
            CameraInput {
                matrix,
                matrix_buffer: None,
                field,
                continuous: true,
                mapping: Mapping::Originals,
            },
            input,
        )
    }

    pub(crate) fn from_resident(
        device: &wgpu::Device,
        renderer: &crate::renderer::GpuRenderer,
        source: &ProcessingSource,
        input: &ProcessingInput,
        raster: crate::gpu::raster::Raster,
    ) -> Result<Self, Error> {
        input.require_source(source)?;
        if u64::from(raster.size()[0]) * u64::from(raster.size()[1])
            != u64::from(source.pixel_count())
        {
            return Err(Error::InvalidParameter(
                "processing source layout differs from plan",
            ));
        }
        #[cfg(feature = "raw")]
        if !input.repairs.0.is_empty() && source.geometry().is_none() {
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
            renderer,
            camera,
            CameraInput {
                matrix: source.camera_to_ap1,
                matrix_buffer: Some(source.matrix.clone()),
                field: source.flat_field.clone(),
                continuous: source.continuous,
                mapping: Mapping::Resident(source.geometry()),
            },
            input,
        )
    }

    fn capture(
        device: &wgpu::Device,
        renderer: &crate::renderer::GpuRenderer,
        source: Arc<ImageNode>,
        definition: CameraInput,
        input: &ProcessingInput,
    ) -> Result<Self, Error> {
        let matrix = definition.matrix;
        let mapping = definition.mapping;
        if let Some(pipeline) = input.pipeline() {
            if pipeline.input_program().matrix != matrix {
                return Err(Error::InvalidParameter(
                    "photographic conversion matrix differs from captured input",
                ));
            }
            #[cfg(feature = "raw")]
            pipeline.validate_repairs(device, input.repairs.bound())?;
        }

        let captured = Captured::new(
            device,
            renderer,
            definition,
            input
                .pipeline()
                .map(|pipeline| pipeline.input_program().clone()),
            (!input.parameters().graded()).then_some(input.parameters()),
        )?;
        let image = |output| {
            ImageNode::new(
                Domain {
                    meaning: if matches!(output, Output::Neutral) {
                        Meaning::Neutral
                    } else {
                        Meaning::Graded
                    },
                    ..source.domain
                },
                Some(source.clone()),
                Camera {
                    captured: captured.clone(),
                    output,
                },
            )
        };
        let unrepaired = image(Output::Neutral);
        #[cfg(feature = "raw")]
        let neutral = if input.parameters().graded() && !input.repairs.0.is_empty() {
            ImageNode::new(
                unrepaired.domain,
                Some(unrepaired.clone()),
                Repairs {
                    snapshot: input.repairs.clone(),
                    code: captured.code.clone(),
                    mapping,
                },
            )
        } else {
            unrepaired.clone()
        };
        #[cfg(not(feature = "raw"))]
        let neutral = unrepaired.clone();

        if !input.parameters().graded() {
            return Ok(Self {
                camera: source,
                unrepaired,
                roll: neutral.clone(),
                graded: neutral.clone(),
                neutral,
            });
        }

        let photographic = Arc::new(Photographic {
            parameters: input.parameters(),
            grade: input.pipeline().map(|pipeline| pipeline.grade.clone()),
            #[cfg(feature = "raw")]
            repairs: input.repairs.clone(),
        });
        let grading = if let Some(grade) = input.grade() {
            Grading::new(device, grade, 0)
        } else {
            Grading::from_parts(
                device,
                input.parameters(),
                captured.empty_layers.clone(),
                captured.empty_context.clone(),
                false,
                0,
            )
        };
        let grading = match mapping {
            Mapping::Resident(geometry) => grading.with_camera_coordinates(geometry),
            #[cfg(feature = "raw")]
            Mapping::Originals => grading,
        };

        let roll = ImageNode::implemented_by(
            grading.roll(neutral.clone())?,
            image(Output::Roll(photographic.clone())),
        )?;
        let graded = ImageNode::implemented_by(
            grading.layers(roll.clone())?,
            image(Output::Graded(photographic)),
        )?;

        Ok(Self {
            camera: source,
            unrepaired,
            neutral,
            roll,
            graded,
        })
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
        renderer: &crate::renderer::GpuRenderer,
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
            renderer,
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
        renderer: &crate::renderer::GpuRenderer,
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
            renderer,
            image,
            CameraInput {
                matrix: self.matrix,
                matrix_buffer: Some(source.matrix.clone()),
                field: source.flat_field.clone(),
                continuous: source.continuous,
                mapping: Mapping::Resident(source.geometry()),
            },
        )
    }

    fn compose(
        &self,
        device: &wgpu::Device,
        renderer: &crate::renderer::GpuRenderer,
        source: Arc<ImageNode>,
        definition: CameraInput,
    ) -> Result<Arc<ImageNode>, Error> {
        let mapping = definition.mapping;
        let captured = Captured::new(device, renderer, definition, None, Some(self.parameters))?;
        let image = ImageNode::new(
            Domain {
                meaning: Meaning::Neutral,
                ..source.domain
            },
            Some(source),
            Camera {
                captured: captured.clone(),
                output: Output::Neutral,
            },
        );
        Ok(if self.repairs.0.is_empty() {
            image
        } else {
            ImageNode::new(
                image.domain,
                Some(image),
                Repairs {
                    snapshot: self.repairs.clone(),
                    code: captured.code.clone(),
                    mapping,
                },
            )
        })
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
        let matrix = capture.matrix_buffer.as_ref().map_or_else(
            || f.uniform(&capture.matrix),
            |buffer| Binding::Shared(Shared::Buffer(buffer.clone())),
        );
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
        #[cfg(feature = "raw")]
        {
            let photo = match &self.output {
                Output::Neutral => return Ok(()),
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

#[cfg(feature = "raw")]
fn repair_geometry(
    geometry: Option<SourceGeometry>,
) -> Option<(crate::geometry::SourceRegion, SourceGeometry)> {
    let geometry = geometry?;
    let region = geometry.region();
    (geometry.size() == [region.width, region.height]).then_some((region, geometry))
}

#[cfg(feature = "raw")]
struct Repairs {
    snapshot: crate::gpu::repairs::Snapshot,
    code: Arc<str>,
    mapping: Mapping,
}

#[cfg(feature = "raw")]
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
