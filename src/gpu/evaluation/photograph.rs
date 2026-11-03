//! Photographic image boundaries and their contracted camera-cell implementations.

use super::*;
use crate::gpu::{
    GpuFlatField, ProcessingInput, ProcessingParameters, ProcessingSource, SourceGeometry,
    flat::FieldGeometry,
};

mod assembly;
mod resident;
use resident::ResidentCamera;
#[cfg(any(feature = "ui", feature = "editor"))]
mod prewarm;
#[cfg(test)]
pub(crate) mod test_support;

use crate::gpu::photograph::Boundary;
use assembly::{Assembly, Interpretation};

/// Captured camera, neutral and photographic image boundaries.
/// Each boundary is calculated by its captured operations.
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

    fn geometry(self, region: CellRegion) -> Result<Option<SourceGeometry>, Error> {
        match self {
            Self::Originals => {
                let r = region.source()?;
                SourceGeometry::original_tile([r.x, r.y, r.width, r.height], [r.width, r.height])
                    .map(Some)
            }
            Self::Resident(geometry) => Ok(geometry),
        }
    }
}

struct CameraInput {
    matrix: glam::DMat3,
    field: Option<Arc<GpuFlatField>>,
    continuous: bool,
    mapping: Mapping,
}

struct Captured {
    interpretation: Interpretation,
    matrix: glam::DMat3,
    field: Option<Arc<GpuFlatField>>,
    mapping: Mapping,
    code: Arc<str>,
}

impl Captured {
    fn new(definition: CameraInput, interpretation: Interpretation) -> Result<Arc<Self>, Error> {
        let CameraInput {
            matrix,
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
        crate::gpu::CameraMatrix::new(matrix)?;
        Ok(Arc::new(Self {
            interpretation,
            matrix,
            field,
            mapping,
            code: crate::gpu::photograph::module_shader().into(),
        }))
    }

    fn bind_field(&self, function: Function, geometry: &FieldGeometry) -> Function {
        let function = function.with_uniform(5, geometry);
        match &self.field {
            Some(field) => function.with_shared(4, Shared::Buffer(field.nodes.clone())),
            None => function.with_storage(4, bytemuck::bytes_of(&[1f32; 4]).to_vec()),
        }
    }
}

impl Photograph {
    #[cfg(all(
        feature = "raw",
        feature = "project",
        any(feature = "editor", feature = "export")
    ))]
    pub(super) fn unrepaired_readout(
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

        let capture = Captured::new(
            CameraInput {
                matrix: program.matrix,
                field,
                continuous: true,
                mapping: Mapping::Originals,
            },
            Interpretation::Program(program, encoding),
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
    pub(crate) fn resident_camera_bytes(image: &Arc<ImageNode>) -> u64 {
        let mut node = Some(image);
        while let Some(current) = node {
            if let Some(camera) =
                (current.operation.as_ref() as &dyn std::any::Any).downcast_ref::<ResidentCamera>()
            {
                return camera.pixels.size();
            }
            node = current.input.as_ref();
        }
        0
    }

    #[cfg(feature = "editor")]
    pub(crate) fn input_program(image: &Arc<ImageNode>) -> Option<crate::gpu::GpuInputProgram> {
        Assembly::input_program(image)
    }

    #[cfg(feature = "editor")]
    pub(crate) fn filtered(
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
        source: Arc<ImageNode>,
        field: Option<Arc<GpuFlatField>>,
        origins: &[[u32; 2]],
    ) -> Result<Arc<ImageNode>, Error> {
        let height = u32::try_from(origins.len())
            .ok()
            .and_then(|count| count.checked_mul(8))
            .filter(|height| *height > 0)
            .ok_or(Error::InvalidParameter("invalid camera footprint batch"))?;

        if source.domain.meaning != Meaning::CameraCodes
            || source.domain.bounds != Some(CellRegion::new([0, 0], [8, height]))
            || source.domain.grid != Grid::axis_aligned([0.; 2], [1.; 2])
        {
            return Err(Error::InvalidParameter(
                "camera footprints require an original-cell index grid",
            ));
        }

        let captured = Captured::new(
            CameraInput {
                matrix: glam::DMat3::IDENTITY,
                field,
                continuous: true,
                mapping: Mapping::Resident(None),
            },
            Interpretation::Unavailable,
        )?;
        let geometry = if let Some(field) = &captured.field {
            let mapping = SourceGeometry::new(field.field().area(), [1, 1])?;
            FieldGeometry::new(field, mapping)
        } else {
            FieldGeometry::uncorrected(None)
        };
        let function = Function::new(
            format!(
                "{}\n{}",
                crate::gpu::photograph::kernel_shader(),
                include_str!("photograph/gathered.wgsl")
            ),
            "gathered_camera_light",
        )
        .with_storage(18, bytemuck::cast_slice(origins).to_vec());
        let function = captured.bind_field(function, &geometry);
        Ok(pointwise(source, Meaning::Camera, function))
    }

    /// Corrected original camera light, excluding interpretation and recorded positive exposure.
    pub fn camera_light(
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
            CameraInput {
                matrix: glam::DMat3::IDENTITY,
                field,
                continuous: true,
                mapping: Mapping::Originals,
            },
            Interpretation::Unavailable,
        )?;

        Assembly::captured(captured).prefix(source, Boundary::Codes, Boundary::Camera)
    }

    #[cfg(feature = "editor")]
    pub(crate) fn repaired_reconstruction(
        base: &Arc<ImageNode>,
        snapshot: crate::gpu::repairs::Snapshot,
    ) -> Result<Arc<ImageNode>, Error> {
        let input = base.input.as_ref().ok_or(Error::InvalidParameter(
            "repair reconstruction requires an unrepaired neutral base",
        ))?;
        if base.domain.bounds.is_some()
            || base.domain.meaning != Meaning::Neutral
            || input.domain.meaning != Meaning::Neutral
            || input.domain.grid != Grid::axis_aligned([0.; 2], [1.; 2])
        {
            return Err(Error::InvalidParameter(
                "repair reconstruction requires original neutral cells",
            ));
        }
        snapshot.require_area(input.domain.finite()?.source()?)?;
        let repaired = Repairs::image(
            input.clone(),
            snapshot,
            crate::gpu::photograph::module_shader().into(),
            Mapping::Originals,
        );
        Ok(ImageNode::captured(
            base.domain,
            Some(repaired),
            base.operation.clone(),
        ))
    }

    pub fn new(
        limits: &wgpu::Limits,
        source: Arc<ImageNode>,
        field: Option<Arc<GpuFlatField>>,
        input: &ProcessingInput,
        matrix: glam::DMat3,
    ) -> Result<Self, Error> {
        Self::new_with_stages(limits, source, field, input, matrix, &Default::default())
    }

    pub(crate) fn new_with_stages(
        limits: &wgpu::Limits,
        source: Arc<ImageNode>,
        field: Option<Arc<GpuFlatField>>,
        input: &ProcessingInput,
        matrix: glam::DMat3,
        stages: &crate::pixel_pipeline::PixelPipeline,
    ) -> Result<Self, Error> {
        Self::original_assembly(limits, &source, field, input, matrix)?.photograph(source, stages)
    }

    #[cfg(feature = "editor")]
    pub(crate) fn preview(
        limits: &wgpu::Limits,
        scan: &crate::scan::Scan,
        field: Option<Arc<GpuFlatField>>,
        recipe: &crate::image_recipe::ImageRecipe,
        repairs: &crate::gpu::repairs::Snapshot,
        frame: crate::frame_raster::FrameRaster,
        native_grid: Option<crate::frame_raster::FrameRaster>,
    ) -> Result<(Self, crate::acquisition::Plan), Error> {
        let mut input = ProcessingInput::from_recipe(limits, recipe)?;
        input.repairs = repairs.clone();
        let program = input.source_program().ok_or(Error::InvalidParameter(
            "working photograph has no captured source interpretation",
        ))?;
        if !program.matches_native(scan)?
            || scan.flat_field() != recipe.neutral.correction.as_ref()
            || recipe.neutral.correction.as_ref() != field.as_ref().map(|field| field.field())
        {
            return Err(Error::Gpu(
                "physical-view source authority differs from originals".into(),
            ));
        }
        Self::preview_with_input(
            limits,
            super::camera_source(scan)?,
            field,
            &input,
            frame,
            native_grid,
        )
    }

    #[cfg(feature = "editor")]
    pub(crate) fn preview_with_input(
        limits: &wgpu::Limits,
        source: Arc<ImageNode>,
        field: Option<Arc<GpuFlatField>>,
        input: &ProcessingInput,
        frame: crate::frame_raster::FrameRaster,
        native_grid: Option<crate::frame_raster::FrameRaster>,
    ) -> Result<(Self, crate::acquisition::Plan), Error> {
        let program = input.source_program().ok_or(Error::InvalidParameter(
            "working photograph has no captured source interpretation",
        ))?;
        if frame.composition.area() != source.domain.finite()?.source()? {
            return Err(Error::InvalidParameter(
                "frame belongs to a different source grid",
            ));
        }
        Self::original_assembly(limits, &source, field, input, program.matrix)?.preview(
            source,
            &input.pixel_pipeline(),
            frame,
            native_grid,
        )
    }

    fn original_assembly(
        limits: &wgpu::Limits,
        source: &Arc<ImageNode>,
        field: Option<Arc<GpuFlatField>>,
        input: &ProcessingInput,
        matrix: glam::DMat3,
    ) -> Result<Assembly, Error> {
        if source.domain.meaning != Meaning::CameraCodes
            || source.domain.grid != Grid::axis_aligned([0.; 2], [1.; 2])
        {
            return Err(Error::InvalidParameter(
                "photographic conversion requires original camera cells",
            ));
        }
        input
            .repairs
            .require_area(source.domain.finite()?.source()?)?;
        if let Some(grading) = input.grade() {
            grading.require_extent(source.domain.finite()?.source()?)?;
        }
        Self::capture(
            limits,
            CameraInput {
                matrix,
                field,
                continuous: true,
                mapping: Mapping::originals(),
            },
            input,
        )
    }

    pub(crate) fn from_resident_with_stages(
        limits: &wgpu::Limits,
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
        Self::capture(
            limits,
            CameraInput {
                matrix: source.camera_to_ap1,
                field: source.flat_field.clone(),
                continuous: source.continuous,
                mapping: Mapping::Resident(source.geometry),
            },
            input,
        )?
        .photograph(camera, stages)
    }

    fn capture(
        limits: &wgpu::Limits,
        definition: CameraInput,
        input: &ProcessingInput,
    ) -> Result<Assembly, Error> {
        let matrix = definition.matrix;
        let mapping = definition.mapping;
        if let Some(program) = input.source_program() {
            if program.matrix != matrix {
                return Err(Error::InvalidParameter(
                    "photographic conversion matrix differs from captured input",
                ));
            }
            input.validate_repairs(limits, input.repairs.bound())?;
        }

        let captured = Captured::new(
            definition,
            match input.source_program() {
                Some(program) => Interpretation::Program(*program, Default::default()),
                None if !input.is_positive() => Interpretation::Packed(input.parameters()),
                None => Interpretation::Unavailable,
            },
        )?;
        let mut assembly = Assembly::captured(captured.clone());
        if input.parameters().graded() {
            assembly.repairs = input.repairs.clone();
        }

        if input.parameters().graded() {
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

        Ok(assembly)
    }
}

/// Adapter-captured neutral conversion, with no downstream photographic grade ownership.
#[cfg(feature = "editor")]
pub(crate) struct Neutral {
    pub interpretation: crate::gpu::SourceInterpretation,
    pub field: Option<Arc<GpuFlatField>>,
    pub repairs: crate::gpu::repairs::Snapshot,
}

#[cfg(feature = "editor")]
impl Neutral {
    pub fn originals(&self, source: Arc<ImageNode>) -> Result<Arc<ImageNode>, Error> {
        if source.domain.meaning != Meaning::CameraCodes
            || source.domain.grid != Grid::axis_aligned([0.; 2], [1.; 2])
        {
            return Err(Error::InvalidParameter(
                "neutral conversion requires original camera cells",
            ));
        }
        self.repairs
            .require_area(source.domain.finite()?.source()?)?;
        self.compose(
            source,
            CameraInput {
                matrix: self.interpretation.matrix(),
                field: self.field.clone(),
                continuous: true,
                mapping: Mapping::Originals,
            },
        )
    }

    pub fn resident(
        &self,
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
        if source.camera_to_ap1 != self.interpretation.matrix() {
            return Err(Error::InvalidParameter(
                "neutral source matrix differs from captured input",
            ));
        }
        let domain = Domain {
            bounds: Some(crate::gpu::evaluation::CellRegion::new(
                [0, 0],
                raster.size(),
            )),
            grid: Grid::from_raster(raster),
            meaning: Meaning::CameraCodes,
        };
        let image = ImageNode::new(
            domain,
            None,
            ResidentCamera {
                domain,
                pixels: source.input.clone(),
                continuous: source.continuous,
            },
        );
        self.compose(
            image,
            CameraInput {
                matrix: self.interpretation.matrix(),
                field: self.field.clone(),
                continuous: source.continuous,
                mapping: Mapping::Resident(source.geometry),
            },
        )
    }

    #[cfg(feature = "editor")]
    pub(crate) fn recompose_resident(
        &self,
        image: &Arc<ImageNode>,
    ) -> Result<Arc<ImageNode>, Error> {
        let mut node = Some(image);
        while let Some(current) = node {
            if let Some(camera) =
                (current.operation.as_ref() as &dyn std::any::Any).downcast_ref::<Camera>()
                && let Some(input) = &current.input
                && let Some(resident) = (input.operation.as_ref() as &dyn std::any::Any)
                    .downcast_ref::<ResidentCamera>()
            {
                let root = self.compose(
                    input.clone(),
                    CameraInput {
                        matrix: self.interpretation.matrix(),
                        field: self.field.clone(),
                        continuous: resident.continuous,
                        mapping: camera.captured.mapping,
                    },
                )?;
                return reconstruct(root, image.domain);
            }
            node = current.input.as_ref();
        }
        Err(Error::InvalidParameter(
            "calculation has no resident camera argument",
        ))
    }

    fn compose(
        &self,
        source: Arc<ImageNode>,
        definition: CameraInput,
    ) -> Result<Arc<ImageNode>, Error> {
        let captured = Captured::new(
            definition,
            match self.interpretation {
                crate::gpu::SourceInterpretation::Photograph(program) => {
                    Interpretation::Program(program, Default::default())
                }
                crate::gpu::SourceInterpretation::Inspection(_) => {
                    Interpretation::Packed(ProcessingParameters::source())
                }
            },
        )?;
        let mut assembly = Assembly::captured(captured);
        assembly.repairs = self.repairs.clone();

        assembly.prefix(source, Boundary::Codes, Boundary::Neutral)
    }
}

struct Camera {
    captured: Arc<Captured>,
}

#[derive(Debug, PartialEq)]
struct CameraIdentity {
    field: Option<Arc<crate::flat_field::FlatField>>,
    original_mapping: bool,
    geometry: Option<SourceGeometry>,
}

impl Operation for Camera {
    fn sample_producer(&self) -> Option<&dyn SampleProducer> {
        Some(self)
    }

    fn pointwise(&self) -> Option<Function> {
        if !matches!(self.captured.mapping, Mapping::Originals) {
            return None;
        }
        let capture = &self.captured;
        let geometry = match &capture.field {
            Some(field) => FieldGeometry::new(
                field,
                SourceGeometry::new(field.field().area(), [1, 1]).ok()?,
            ),
            None => FieldGeometry::uncorrected(None),
        };
        Some(capture.bind_field(
            Function::new(
                format!(
                    "{}\n{}",
                    crate::gpu::photograph::pixel_shader(),
                    r#"
fn corrected_camera_light(pixel: vec4f, position: vec2f) -> vec4f {
    return vec4f(camera_light_at(pixel.rgb, vec2u(floor(position))), 0.0);
}
"#
                ),
                "corrected_camera_light",
            ),
            &geometry,
        ))
    }

    fn identity(&self) -> Option<RetainedKey> {
        let (original_mapping, geometry) = match self.captured.mapping {
            Mapping::Originals => (true, None),
            Mapping::Resident(geometry) => (false, geometry),
        };
        Some(RetainedKey::new(CameraIdentity {
            field: self
                .captured
                .field
                .as_ref()
                .map(|field| field.field.clone()),
            original_mapping,
            geometry,
        }))
    }

    fn input(&self, region: CellRegion) -> Result<Option<CellRegion>, Error> {
        Ok(Some(region))
    }

    fn lower(
        &self,
        f: &mut Fragment,
        input: Option<PlannedImage>,
        output: PlannedImage,
    ) -> Result<(), Error> {
        let sample = Sampled::raster(f, input.unwrap());
        self.emit(
            f,
            Some(SampleInput::Callable(sample)),
            output.domain,
            output.region,
        )?
        .materialize(f, output)
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

impl Repairs {
    fn image(
        input: Arc<ImageNode>,
        snapshot: crate::gpu::repairs::Snapshot,
        code: Arc<str>,
        mapping: Mapping,
    ) -> Arc<ImageNode> {
        if snapshot.0.is_empty() {
            return input;
        }
        ImageNode::new(
            input.domain,
            Some(input),
            Self {
                snapshot,
                code,
                mapping,
            },
        )
    }
}

impl Operation for Repairs {
    fn identity(&self) -> Option<RetainedKey> {
        #[derive(Debug, PartialEq)]
        struct Repaired(RetainedKey, bool, Option<SourceGeometry>);
        Some(RetainedKey::new(Repaired(
            RetainedKey::new(self.snapshot.content()),
            matches!(self.mapping, Mapping::Originals),
            match self.mapping {
                Mapping::Originals => None,
                Mapping::Resident(geometry) => geometry,
            },
        )))
    }

    fn input(&self, region: CellRegion) -> Result<Option<CellRegion>, Error> {
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
        let Some((region, geometry)) = repair_geometry(self.mapping.geometry(output.region)?)
        else {
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
