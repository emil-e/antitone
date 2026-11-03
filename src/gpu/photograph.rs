//! Photographic image boundaries and their contracted camera-cell implementations.

use crate::Error;
use crate::gpu::evaluation::*;
use crate::gpu::flat::FieldGeometry;
use crate::gpu::flat::GpuFlatField;
use crate::gpu::grading::Grading;
use crate::gpu::input::ProcessingInput;
use crate::gpu::parameters::ProcessingParameters;
use crate::gpu::source::ProcessingSource;
use crate::gpu::source::SourceGeometry;
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
pub mod neutral_readout;
mod operations;
pub(crate) mod shader;
#[cfg(all(test, feature = "editor"))]
mod tests;
use operations::{Camera, CameraInput, Captured, Interpretation, Mapping, Repairs, ResidentCamera};
#[cfg(any(feature = "ui", feature = "editor"))]
mod prewarm;
#[cfg(test)]
pub mod test_support;

use assembly::{Assembly, Boundary};

/// Captured camera, neutral and photographic image boundaries.
/// Each boundary is calculated by its captured operations.
pub struct PhotographGraph {
    pub camera: Arc<ImageNode>,
    pub unrepaired: Arc<ImageNode>,
    pub neutral: Arc<ImageNode>,
    pub roll: Arc<ImageNode>,
    pub graded: Arc<ImageNode>,
    pub(crate) processed: Arc<ImageNode>,
}

impl PhotographGraph {
    #[cfg(all(
        feature = "raw",
        feature = "project",
        any(feature = "editor", feature = "export")
    ))]
    pub(super) fn unrepaired_readout(
        source: Arc<ImageNode>,
        field: Option<Arc<GpuFlatField>>,
        program: crate::gpu::interpretation::InputTransform,
        encoding: crate::processing::RollSettings,
    ) -> Result<Arc<ImageNode>, Error> {
        if source.domain().meaning != Meaning::CameraCodes
            || source.domain().grid != Grid::axis_aligned([0.; 2], [1.; 2])
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
        program: crate::gpu::interpretation::InputTransform,
        encoding: crate::processing::RollSettings,
    ) -> Result<Arc<ImageNode>, Error> {
        Assembly::native(program, encoding).prefix(camera, Boundary::Camera, Boundary::Native)
    }

    #[cfg(feature = "editor")]
    pub(crate) fn resident_camera_bytes(image: &Arc<ImageNode>) -> u64 {
        let mut node = Some(image);
        while let Some(current) = node {
            if let Some(camera) = (current.operation().as_ref() as &dyn std::any::Any)
                .downcast_ref::<ResidentCamera>()
            {
                return camera.pixels.size();
            }
            node = current.input();
        }
        0
    }

    #[cfg(feature = "editor")]
    pub(crate) fn input_transform(
        image: &Arc<ImageNode>,
    ) -> Option<crate::gpu::interpretation::InputTransform> {
        Assembly::input_transform(image)
    }

    #[cfg(feature = "editor")]
    pub(crate) fn filtered(
        grading: &Grading,
        neutral: Arc<ImageNode>,
        stages: &crate::pixel_pipeline::PixelPipeline,
        presentation: crate::frame_raster::FrameRaster,
        processing: crate::frame_raster::FrameRaster,
    ) -> Result<Arc<ImageNode>, Error> {
        let output = view_domain(presentation, neutral.domain().grid.coverage);
        let scale = std::array::from_fn(|i| {
            f64::from(processing.output[i]) / f64::from(presentation.output[i])
        });
        Assembly::filtered(grading.clone()).filtered_image(neutral, stages, output, [0; 2], scale)
    }

    #[cfg(feature = "editor")]
    pub(crate) fn inspection_view(
        neutral: Arc<ImageNode>,
        parameters: ProcessingParameters,
        presentation: crate::frame_raster::FrameRaster,
    ) -> Result<Arc<ImageNode>, Error> {
        let output = view_domain(presentation, neutral.domain().grid.coverage);
        window(inspect(neutral, parameters)?, output.finite()?)
    }

    #[cfg(feature = "editor")]
    pub(crate) fn camera_footprints(
        source: Arc<ImageNode>,
        field: Option<Arc<GpuFlatField>>,
        origins: &[[u32; 2]],
    ) -> Result<Arc<ImageNode>, Error> {
        Self::camera_patches(source, field, origins, [8, 8])
    }

    /// Individually selected original cells, retaining their physical correction coordinates.
    #[cfg(feature = "editor")]
    pub(crate) fn camera_samples(
        source: Arc<ImageNode>,
        field: Option<Arc<GpuFlatField>>,
        positions: &[[u32; 2]],
    ) -> Result<Arc<ImageNode>, Error> {
        Self::camera_patches(source, field, positions, [1, 1])
    }

    #[cfg(feature = "editor")]
    fn camera_patches(
        source: Arc<ImageNode>,
        field: Option<Arc<GpuFlatField>>,
        origins: &[[u32; 2]],
        patch: [u32; 2],
    ) -> Result<Arc<ImageNode>, Error> {
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
    pub fn camera_light(
        source: Arc<ImageNode>,
        field: Option<Arc<GpuFlatField>>,
    ) -> Result<Arc<ImageNode>, Error> {
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

        Assembly::captured(captured).prefix(source, Boundary::Codes, Boundary::Camera)
    }

    #[cfg(feature = "editor")]
    pub(crate) fn repaired_reconstruction(
        base: &Arc<ImageNode>,
        snapshot: crate::gpu::repairs::Snapshot,
    ) -> Result<Arc<ImageNode>, Error> {
        let input = base.input().ok_or(Error::InvalidParameter(
            "repair reconstruction requires an unrepaired neutral base",
        ))?;
        if base.domain().bounds.is_some()
            || base.domain().meaning != Meaning::Neutral
            || input.domain().meaning != Meaning::Neutral
            || input.domain().grid != Grid::axis_aligned([0.; 2], [1.; 2])
        {
            return Err(Error::InvalidParameter(
                "repair reconstruction requires original neutral cells",
            ));
        }
        snapshot.require_area(input.domain().finite()?.source()?)?;
        let repaired = Repairs::image(
            input.clone(),
            snapshot,
            crate::gpu::repairs::shader().into(),
            Mapping::Originals,
        );
        Ok(ImageNode::captured(
            base.domain(),
            Some(repaired),
            base.operation().clone(),
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
        let program = input.source_transform().ok_or(Error::InvalidParameter(
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
            crate::gpu::evaluation::camera_source(scan)?,
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

    fn original_assembly(
        limits: &wgpu::Limits,
        source: &Arc<ImageNode>,
        field: Option<Arc<GpuFlatField>>,
        input: &ProcessingInput,
        matrix: glam::DMat3,
    ) -> Result<Assembly, Error> {
        if source.domain().meaning != Meaning::CameraCodes
            || source.domain().grid != Grid::axis_aligned([0.; 2], [1.; 2])
        {
            return Err(Error::InvalidParameter(
                "photographic conversion requires original camera cells",
            ));
        }
        input
            .repairs
            .require_area(source.domain().finite()?.source()?)?;
        if let Some(grading) = input.grade() {
            grading.require_extent(source.domain().finite()?.source()?)?;
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
        if let Some(program) = input.source_transform() {
            if program.matrix != matrix {
                return Err(Error::InvalidParameter(
                    "photographic conversion matrix differs from captured input",
                ));
            }
            input.validate_repairs(limits, input.repairs.bound())?;
        }

        let captured = Captured::new(
            definition,
            match input.source_transform() {
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
    pub repairs: crate::gpu::repairs::Snapshot,
}

#[cfg(feature = "editor")]
impl NeutralInput {
    pub fn original_views(
        &self,
        source: Arc<ImageNode>,
        frames: impl IntoIterator<Item = crate::frame_raster::FrameRaster>,
    ) -> Result<Vec<Arc<ImageNode>>, Error> {
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
                        meaning: Meaning::Neutral,
                    },
                )
            })
            .collect()
    }

    pub fn resident_view(
        &self,
        source: &ProcessingSource,
    ) -> Result<Arc<crate::gpu::evaluation::ImageNode>, Error> {
        let geometry = source
            .geometry()
            .ok_or(Error::InvalidParameter("neutral source requires geometry"))?;
        let raster = crate::gpu::raster::Raster::region(geometry.region(), geometry.size());
        let root = self.resident(source, raster)?;
        let output_domain = crate::gpu::evaluation::Domain {
            grid: crate::gpu::evaluation::Grid {
                coverage: 8,
                ..root.domain().grid
            },
            ..root.domain()
        };
        crate::gpu::evaluation::reconstruct(root, output_domain)
    }

    pub fn originals(&self, source: Arc<ImageNode>) -> Result<Arc<ImageNode>, Error> {
        if source.domain().meaning != Meaning::CameraCodes
            || source.domain().grid != Grid::axis_aligned([0.; 2], [1.; 2])
        {
            return Err(Error::InvalidParameter(
                "neutral conversion requires original camera cells",
            ));
        }
        self.repairs
            .require_area(source.domain().finite()?.source()?)?;
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
                (current.operation().as_ref() as &dyn std::any::Any).downcast_ref::<Camera>()
                && let Some(input) = current.input()
                && let Some(resident) = (input.operation().as_ref() as &dyn std::any::Any)
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
                return reconstruct(root, image.domain());
            }
            node = current.input();
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
                crate::gpu::interpretation::SourceInterpretation::Photograph(program) => {
                    Interpretation::Program(program, Default::default())
                }
                crate::gpu::interpretation::SourceInterpretation::Inspection(_) => {
                    Interpretation::Packed(ProcessingParameters::source())
                }
            },
        )?;
        let mut assembly = Assembly::captured(captured);
        assembly.repairs = self.repairs.clone();

        assembly.prefix(source, Boundary::Codes, Boundary::Neutral)
    }
}
