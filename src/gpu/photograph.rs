//! Photographic image boundaries and their contracted camera-cell implementations.

use crate::Error;
use crate::gpu::evaluation::*;
#[cfg(any(test, feature = "editor"))]
use crate::gpu::flat::FieldGeometry;
use crate::gpu::flat::GpuFlatField;
use crate::gpu::grading::Grading;
#[cfg(any(test, feature = "editor"))]
use crate::gpu::parameters::ProcessingParameters;
use crate::gpu::source::ProcessingSource;
#[cfg(any(test, feature = "editor"))]
use crate::gpu::source::SourceGeometry;
#[cfg(test)]
use crate::gpu::test_inputs::AuthoredPhotograph;
use std::sync::Arc;

pub(crate) mod admission;
mod assembly;
#[cfg(feature = "editor")]
pub mod camera_response;
#[cfg(all(
    feature = "raw",
    feature = "project",
    any(feature = "editor", feature = "export")
))]
#[cfg(all(test, feature = "editor"))]
mod neutral_readout;
pub(crate) mod operations;
#[cfg(feature = "project")]
mod recipe;
#[cfg(feature = "project")]
#[cfg(feature = "project")]
pub(crate) use recipe::RecipeGraph;
#[cfg(all(test, feature = "editor"))]
mod provenance_tests;
#[cfg(feature = "editor")]
pub(crate) mod request;
#[cfg(all(feature = "ui", feature = "render"))]
mod resident;
pub(crate) mod shader;
#[cfg(all(test, feature = "editor"))]
mod tests;
use operations::{Camera, CameraInput, Captured, Interpretation, Mapping, ResidentCamera};
#[cfg(all(feature = "render", any(feature = "ui", feature = "editor")))]
mod prewarm;
#[cfg(test)]
pub mod test_support;

use assembly::Assembly;

/// Corrected camera light is available independently of positive calibration or grading.
pub(crate) struct CameraGraph {
    pub codes: Value,
    pub camera: Value,
    input: CameraInput,
}

impl CameraGraph {
    fn new(codes: Value, input: CameraInput) -> Result<Self, Error> {
        if codes.domain().meaning != Meaning::CameraCodes {
            return Err(Error::InvalidParameter(
                "camera graph requires camera codes",
            ));
        }
        let capture = Captured::new(input.clone(), Interpretation::Unavailable)?;
        let camera = Assembly::captured(capture).camera(codes.clone());
        Ok(Self {
            codes,
            camera,
            input,
        })
    }

    fn inspection(&self, view: crate::gpu::interpretation::InspectionView) -> Result<Value, Error> {
        Ok(Assembly::captured(Captured::new(
            self.input.clone(),
            Interpretation::Packed(view.parameters()?),
        )?)
        .interpret_camera(self.codes.clone(), self.camera.clone())?
        .unrepaired)
    }
}

/// Captured camera, neutral and photographic image boundaries.
/// Each boundary is calculated by its captured operations.
pub struct PhotographGraph {
    pub codes: Value,
    pub camera: Value,
    pub camera_positive: Value,
    pub unrepaired: Value,
    pub neutral: Value,
    pub roll: Value,
    pub graded: Value,
    pub(crate) processed: Value,
}

impl PhotographGraph {
    #[cfg(feature = "editor")]
    pub(super) fn native_positive(
        camera: Value,
        program: crate::gpu::interpretation::InputTransform,
    ) -> Result<Value, Error> {
        Assembly::native(program).interpret(camera)
    }

    #[cfg(feature = "editor")]
    pub(crate) fn resident_camera_bytes(image: &Value) -> u64 {
        let mut buffers = Vec::new();
        for node in image.ancestors() {
            if let Some(camera) =
                (node.operation().as_ref() as &dyn std::any::Any).downcast_ref::<ResidentCamera>()
                && !buffers.contains(&&camera.pixels)
            {
                buffers.push(&camera.pixels);
            }
        }
        buffers.into_iter().map(wgpu::Buffer::size).sum()
    }

    #[cfg(feature = "editor")]
    pub(crate) fn has_correction(image: &Value, field: Option<&Arc<GpuFlatField>>) -> bool {
        let mut found = false;
        for node in image.ancestors() {
            if let Some(camera) = node.parameters::<operations::CameraIdentity>() {
                if camera.field.as_ref() != field.map(|field| field.field()) {
                    return false;
                }
                found = true;
            }
        }
        found
    }

    #[cfg(feature = "editor")]
    pub(crate) fn input_transform(
        image: &Value,
    ) -> Result<Option<crate::gpu::interpretation::InputTransform>, Error> {
        Assembly::input_transform(image)
    }

    #[cfg(all(test, feature = "editor"))]
    pub(crate) fn filtered(
        grading: &Grading,
        neutral: Value,
        stages: &crate::pixel_pipeline::PixelPipeline,
        presentation: crate::frame_raster::FrameRaster,
        processing: crate::frame_raster::FrameRaster,
    ) -> Result<Value, Error> {
        let output = view_domain(presentation, neutral.domain().grid.coverage);
        let scale = std::array::from_fn(|i| {
            f64::from(processing.output[i]) / f64::from(presentation.output[i])
        });
        Assembly::filtered(grading.clone()).filtered_image(neutral, stages, output, [0; 2], scale)
    }

    #[cfg(feature = "editor")]
    pub(crate) fn inspection_view(
        neutral: Value,
        parameters: ProcessingParameters,
        presentation: crate::frame_raster::FrameRaster,
    ) -> Result<Value, Error> {
        let output = view_domain(presentation, neutral.domain().grid.coverage);
        window(inspect(neutral, parameters)?, output.finite()?)
    }

    /// Individually selected original cells, retaining their physical correction coordinates.
    #[cfg(feature = "editor")]
    pub(crate) fn camera_samples(
        source: Value,
        field: Option<Arc<GpuFlatField>>,
        positions: &[[u32; 2]],
    ) -> Result<Value, Error> {
        Self::camera_patches(source, field, positions, [1, 1])
    }

    #[cfg(feature = "editor")]
    fn camera_patches(
        source: Value,
        field: Option<Arc<GpuFlatField>>,
        origins: &[[u32; 2]],
        patch: [u32; 2],
    ) -> Result<Value, Error> {
        let height = u32::try_from(origins.len())
            .ok()
            .and_then(|count| count.checked_mul(patch[1]))
            .filter(|height| *height > 0)
            .ok_or(Error::InvalidParameter("invalid camera footprint batch"))?;

        if source.domain().meaning != Meaning::CameraCodes
            || source.domain().bounds != Some(CellRegion::new([0, 0], [patch[0], height]))
            || source.domain().grid != Grid::axis_aligned([0.; 2], [1.; 2])
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
                crate::gpu::photograph::shader::functions(),
                include_str!("photograph/gathered.wgsl")
            ),
            "gathered_camera_light",
        )
        .with_storage(18, bytemuck::cast_slice(origins).to_vec())
        .with_uniform(19, &[patch[0], patch[1], 0u32, 0u32]);
        let function = captured.bind_field(function, &geometry);
        Ok(pointwise(source, Meaning::Camera, function))
    }

    /// Corrected original camera light, excluding interpretation and recorded positive exposure.
    pub fn camera_light(source: Value, field: Option<Arc<GpuFlatField>>) -> Result<Value, Error> {
        if source.domain().meaning != Meaning::CameraCodes
            || source.domain().grid != Grid::axis_aligned([0.; 2], [1.; 2])
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

        Ok(Assembly::captured(captured).camera(source))
    }

    /// Capture original-cell operations; these arguments are also used by analytic consumers.
    /// Author a complete original-grid working-positive calculation from operation arguments.
    pub fn original_image(
        limits: &wgpu::Limits,
        source: Value,
        field: Option<Arc<GpuFlatField>>,
        program: crate::gpu::interpretation::InputTransform,
        grading: Grading,
        repairs: &[crate::dust_repair::Repair],
        stages: &crate::pixel_pipeline::PixelPipeline,
    ) -> Result<Value, Error> {
        Ok(Self::original(
            limits,
            source,
            field,
            program,
            grading,
            repairs.to_vec(),
            stages,
        )?
        .processed)
    }

    pub(crate) fn original(
        limits: &wgpu::Limits,
        source: Value,
        field: Option<Arc<GpuFlatField>>,
        program: crate::gpu::interpretation::InputTransform,
        grading: Grading,
        repairs: Vec<crate::dust_repair::Repair>,
        stages: &crate::pixel_pipeline::PixelPipeline,
    ) -> Result<Self, Error> {
        if source.domain().meaning != Meaning::CameraCodes
            || source.domain().grid != Grid::axis_aligned([0.; 2], [1.; 2])
        {
            return Err(Error::InvalidParameter(
                "photographic conversion requires original camera cells",
            ));
        }
        grading.require_extent(source.domain().finite()?.source()?)?;
        Assembly::photographic(
            limits,
            CameraInput {
                matrix: program.matrix,
                field,
                continuous: true,
                mapping: Mapping::Originals,
            },
            program,
            grading,
            repairs,
        )?
        .photograph(source, stages)
    }

    pub(crate) fn inspection_original(
        source: Value,
        field: Option<Arc<GpuFlatField>>,
        matrix: glam::DMat3,
        view: crate::gpu::interpretation::InspectionView,
    ) -> Result<Value, Error> {
        if source.domain().meaning != Meaning::CameraCodes
            || source.domain().grid != Grid::axis_aligned([0.; 2], [1.; 2])
        {
            return Err(Error::InvalidParameter(
                "inspection requires original camera cells",
            ));
        }
        CameraGraph::new(
            source,
            CameraInput {
                matrix,
                field,
                continuous: true,
                mapping: Mapping::Originals,
            },
        )?
        .inspection(view)
    }

    #[cfg(test)]
    pub(crate) fn new(
        limits: &wgpu::Limits,
        source: Value,
        field: Option<Arc<GpuFlatField>>,
        input: &AuthoredPhotograph,
        matrix: glam::DMat3,
    ) -> Result<Self, Error> {
        Self::new_with_stages(limits, source, field, input, matrix, &Default::default())
    }

    #[cfg(test)]
    pub(crate) fn new_with_stages(
        limits: &wgpu::Limits,
        source: Value,
        field: Option<Arc<GpuFlatField>>,
        input: &AuthoredPhotograph,
        matrix: glam::DMat3,
        stages: &crate::pixel_pipeline::PixelPipeline,
    ) -> Result<Self, Error> {
        Self::original_assembly(limits, &source, field, input, matrix)?.photograph(source, stages)
    }

    #[cfg(all(test, feature = "editor"))]
    pub(crate) fn preview_with_input(
        limits: &wgpu::Limits,
        source: Value,
        field: Option<Arc<GpuFlatField>>,
        input: &AuthoredPhotograph,
        frame: crate::frame_raster::FrameRaster,
        native_grid: Option<crate::frame_raster::FrameRaster>,
    ) -> Result<(Self, crate::acquisition::Plan), Error> {
        let program = input.source_transform().ok_or(Error::InvalidParameter(
            "working photograph has no captured source interpretation",
        ))?;
        if frame.composition.area() != source.domain().finite()?.source()? {
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

    #[cfg(test)]
    fn original_assembly(
        limits: &wgpu::Limits,
        source: &Value,
        field: Option<Arc<GpuFlatField>>,
        input: &AuthoredPhotograph,
        matrix: glam::DMat3,
    ) -> Result<Assembly, Error> {
        if source.domain().meaning != Meaning::CameraCodes
            || source.domain().grid != Grid::axis_aligned([0.; 2], [1.; 2])
        {
            return Err(Error::InvalidParameter(
                "photographic conversion requires original camera cells",
            ));
        }
        if let Some(grading) = input.grade() {
            grading.require_extent(source.domain().finite()?.source()?)?;
        }
        Self::capture(
            limits,
            CameraInput {
                matrix,
                field,
                continuous: true,
                mapping: Mapping::Originals,
            },
            input,
        )
    }

    #[cfg(test)]
    pub(crate) fn from_resident_with_stages(
        limits: &wgpu::Limits,
        source: &ProcessingSource,
        input: &AuthoredPhotograph,
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
        if (!input.repairs.is_empty() || !input.replacements.is_empty())
            && source.geometry().is_none()
        {
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
        Self::capture(
            limits,
            CameraInput {
                matrix: source.camera_matrix(),
                field: input.field.clone(),
                continuous: source.continuous,
                mapping: Mapping::Resident(source.geometry()),
            },
            input,
        )?
        .photograph(camera, stages)
    }

    #[cfg(test)]
    fn capture(
        limits: &wgpu::Limits,
        definition: CameraInput,
        input: &AuthoredPhotograph,
    ) -> Result<Assembly, Error> {
        let matrix = definition.matrix;
        let mapping = definition.mapping;
        if let Some(program) = input.source_transform() {
            if program.matrix != matrix {
                return Err(Error::InvalidParameter(
                    "photographic conversion matrix differs from captured input",
                ));
            }
            input.validate_controls(limits)?;
        }

        let captured = Captured::new(
            definition,
            match input.source_transform() {
                Some(program) => Interpretation::Program(*program),
                None if !input.is_positive() => Interpretation::Packed(input.parameters()),
                None => Interpretation::Unavailable,
            },
        )?;
        let mut assembly = Assembly::captured(captured.clone());
        if input.parameters().graded() {
            assembly.repairs = input.repairs.clone();
            assembly.replacements = input.replacements.clone();
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

#[cfg(feature = "editor")]
fn view_domain(frame: crate::frame_raster::FrameRaster, coverage: u32) -> Domain {
    Domain {
        bounds: Some(CellRegion::new([0, 0], frame.output)),
        grid: Grid {
            coverage,
            ..Grid::from_frame(frame)
        },
        meaning: Meaning::Graded,
    }
}

/// Inputs for calculating ungraded AP1, before Roll and adjustment-layer grading.
#[cfg(feature = "editor")]
pub(crate) struct NeutralInput {
    pub interpretation: crate::gpu::interpretation::SourceInterpretation,
    pub field: Option<Arc<GpuFlatField>>,
    pub repairs: Vec<crate::dust_repair::Repair>,
    #[cfg(test)]
    pub replacements: Vec<crate::gpu::repairs::fixtures::Replacement>,
}

#[cfg(feature = "editor")]
impl NeutralInput {
    pub fn original_views(
        &self,
        source: Value,
        frames: impl IntoIterator<Item = crate::frame_raster::FrameRaster>,
    ) -> Result<Vec<Value>, Error> {
        let neutral = self.originals(source)?;
        frames
            .into_iter()
            .map(|frame| {
                let mut grid = Grid::from_frame(frame);
                grid.coverage = 8;
                reconstruct(
                    neutral.clone(),
                    Domain {
                        bounds: None,
                        grid,
                        meaning: neutral.domain().meaning,
                    },
                )
            })
            .collect()
    }

    pub fn originals(&self, source: Value) -> Result<Value, Error> {
        if source.domain().meaning != Meaning::CameraCodes
            || source.domain().grid != Grid::axis_aligned([0.; 2], [1.; 2])
        {
            return Err(Error::InvalidParameter(
                "neutral conversion requires original camera cells",
            ));
        }
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
    ) -> Result<Value, Error> {
        if u64::from(raster.size()[0]) * u64::from(raster.size()[1])
            != u64::from(source.pixel_count())
        {
            return Err(Error::InvalidParameter(
                "neutral source layout differs from its grid",
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
        let image = Value::from_operation(
            domain,
            None,
            ResidentCamera {
                domain,
                identity: Arc::downgrade(&source.identity),
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
                mapping: Mapping::Resident(source.geometry()),
            },
        )
    }

    #[cfg(feature = "editor")]
    pub(crate) fn recompose_resident(&self, image: &Value) -> Result<Value, Error> {
        let mut argument: Option<(&crate::gpu::evaluation::value::Value, Mapping, bool)> = None;
        for current in image.ancestors() {
            if let Some(camera) =
                (current.operation().as_ref() as &dyn std::any::Any).downcast_ref::<Camera>()
                && let Some(input) = current.unary_input()
                && let Some(resident) = (input.operation().as_ref() as &dyn std::any::Any)
                    .downcast_ref::<ResidentCamera>()
            {
                if argument.is_some_and(|(prior, _, _)| !prior.same_definition(input)) {
                    return Err(Error::InvalidParameter(
                        "calculation mixes resident camera arguments",
                    ));
                }
                argument = Some((input, camera.captured.mapping, resident.continuous));
            }
        }
        let Some((input, mapping, continuous)) = argument else {
            return Err(Error::InvalidParameter(
                "calculation has no resident camera argument",
            ));
        };
        let root = self.compose(
            input.clone(),
            CameraInput {
                matrix: self.interpretation.matrix(),
                field: self.field.clone(),
                continuous,
                mapping,
            },
        )?;
        reconstruct(root, image.domain())
    }

    fn compose(&self, source: Value, definition: CameraInput) -> Result<Value, Error> {
        let captured = Captured::new(
            definition,
            match self.interpretation {
                crate::gpu::interpretation::SourceInterpretation::Photograph(program) => {
                    Interpretation::Program(program)
                }
                crate::gpu::interpretation::SourceInterpretation::Inspection(_) => {
                    Interpretation::Packed(ProcessingParameters::source())
                }
            },
        )?;
        let mut assembly = Assembly::captured(captured);
        assembly.repairs = self.repairs.clone();
        #[cfg(test)]
        {
            assembly.replacements = self.replacements.clone();
        }

        assembly.neutral(source)
    }
}
