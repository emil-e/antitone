//! Lowered photographic controls or directly supplied mathematical processing inputs.

use crate::gpu::grading::Grading;
use crate::gpu::interpretation::InputTransform;
use crate::gpu::parameters::ProcessingParameters;
use crate::gpu::source::ProcessingSource;
use crate::{
    Error,
    adjustment_layers::AdjustmentLayerSnapshot,
    geometry::SourceRegion,
    processing::{InputSettings, RollSettings},
};
use glam::{DMat3, DVec3};
use std::sync::Arc;

/// Captured source and photographic settings. Components prepare their own GPU work.
#[derive(Clone)]
pub struct ProcessingInput {
    processing: Processing,
    luts: crate::looks::LutChain,
    pixel_pipeline: crate::pixel_pipeline::PixelPipeline,
    pub(crate) repairs: super::repairs::Snapshot,
}

#[derive(Clone)]
enum Processing {
    Parameters(Arc<ProcessingParameters>),
    Photograph {
        source: Arc<InputTransform>,
        grading: Arc<Grading>,
        area: SourceRegion,
    },
}

impl Processing {
    fn same_content(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Parameters(a), Self::Parameters(b)) => {
                bytemuck::bytes_of(a.as_ref()) == bytemuck::bytes_of(b.as_ref())
            }
            (
                Self::Photograph {
                    source: a,
                    grading: ag,
                    area: aa,
                },
                Self::Photograph {
                    source: b,
                    grading: bg,
                    area: ba,
                },
            ) => a == b && ag.roll == bg.roll && ag.snapshot() == bg.snapshot() && aa == ba,
            _ => false,
        }
    }
}

impl From<ProcessingParameters> for ProcessingInput {
    fn from(parameters: ProcessingParameters) -> Self {
        Self::capture(Processing::Parameters(Arc::new(parameters)))
    }
}

impl From<Arc<ProcessingInput>> for ProcessingInput {
    fn from(input: Arc<ProcessingInput>) -> Self {
        input.as_ref().clone()
    }
}

impl ProcessingInput {
    fn capture(processing: Processing) -> Self {
        Self {
            processing,
            luts: Default::default(),
            pixel_pipeline: Default::default(),
            repairs: Default::default(),
        }
    }

    pub fn new(
        limits: &wgpu::Limits,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        layers: AdjustmentLayerSnapshot,
        matrix: DMat3,
        area: SourceRegion,
    ) -> Result<Self, Error> {
        Self::compose(
            limits,
            InputTransform::new(input, matrix)?,
            Arc::new(Grading::new(limits, roll, layers, area)?),
            area,
        )
    }

    pub fn compose(
        limits: &wgpu::Limits,
        input: InputTransform,
        grading: Arc<Grading>,
        area: SourceRegion,
    ) -> Result<Self, Error> {
        grading.require_extent(area)?;
        Self::validate_limits(
            limits,
            input.input,
            grading.roll,
            grading.snapshot(),
            input.matrix,
            area,
        )?;
        Ok(Self::capture(Processing::Photograph {
            source: Arc::new(input),
            grading,
            area,
        }))
    }

    pub fn validate(
        device: &wgpu::Device,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        layers: &AdjustmentLayerSnapshot,
        matrix: DMat3,
        area: SourceRegion,
    ) -> Result<(), Error> {
        Self::validate_limits(&device.limits(), input, roll, layers, matrix, area)
    }

    pub fn validate_limits(
        limits: &wgpu::Limits,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        layers: &AdjustmentLayerSnapshot,
        matrix: DMat3,
        area: SourceRegion,
    ) -> Result<(), Error> {
        super::photograph::admission::prepare(
            limits,
            input.into(),
            roll,
            layers,
            matrix,
            area,
            DVec3::ZERO,
        )
        .map(|_| ())
    }

    pub(crate) fn validate_repaired(
        device: &wgpu::Device,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        layers: &AdjustmentLayerSnapshot,
        matrix: DMat3,
        area: SourceRegion,
        bound: DVec3,
    ) -> Result<(), Error> {
        super::photograph::admission::prepare(
            &device.limits(),
            input.into(),
            roll,
            layers,
            matrix,
            area,
            bound,
        )
        .map(|_| ())
    }

    pub(crate) fn validate_repairs(
        &self,
        limits: &wgpu::Limits,
        bound: DVec3,
    ) -> Result<(), Error> {
        if let Processing::Photograph {
            source,
            grading,
            area,
        } = &self.processing
        {
            super::photograph::admission::prepare(
                limits,
                source.input,
                grading.roll,
                grading.snapshot(),
                source.matrix,
                *area,
                bound,
            )?;
        }
        Ok(())
    }

    pub fn input_transform(&self) -> &InputTransform {
        match &self.processing {
            Processing::Photograph { source, .. } => source,
            _ => panic!("source inspection has no photographic input program"),
        }
    }

    pub fn snapshot(&self) -> &AdjustmentLayerSnapshot {
        self.grade()
            .expect("source inspection has no grade")
            .snapshot()
    }

    pub fn byte_len(&self) -> u64 {
        self.grade().map_or(0, |grade| grade.byte_len())
    }

    #[cfg(feature = "editor")]
    pub(crate) fn byte_len_for(layers: &AdjustmentLayerSnapshot) -> u64 {
        let active = layers
            .layers()
            .iter()
            .filter(|layer| layer.is_active())
            .count()
            .max(1);
        (active * std::mem::size_of::<super::grading::AdjustmentLayerParameters>()
            + std::mem::size_of::<super::grading::AdjustmentLayerContext>()) as u64
    }
}

impl ProcessingInput {
    /// Capture all photographic components and source interpretation from one recipe.
    #[cfg(all(
        feature = "project",
        feature = "render",
        any(feature = "raw", feature = "ui"),
        any(feature = "editor", test)
    ))]
    pub(crate) fn from_recipe(
        limits: &wgpu::Limits,
        recipe: &crate::image_recipe::ImageRecipe,
    ) -> Result<Self, Error> {
        let input = recipe.neutral.processing_input()?;
        let matrix = input.positive_matrix(
            DMat3::from_cols_array_2d(&recipe.neutral.source_development.camera_to_ap1),
            recipe
                .neutral
                .reference
                .as_ref()
                .map(|reference| reference.ev()),
        )?;
        Ok(Self::new(
            limits,
            input,
            recipe.roll,
            recipe.grade.clone(),
            matrix,
            recipe.neutral.source_development.active_area.into(),
        )?
        .with_luts(recipe.luts.clone())
        .with_pixel_pipeline(recipe.pixel_pipeline()?))
    }

    /// Rendering is downstream of photographic processing; source inspection bypasses both LUTs.
    pub fn with_luts(mut self, luts: impl Into<crate::looks::LutChain>) -> Self {
        self.luts = if self.is_positive() {
            luts.into()
        } else {
            Default::default()
        };
        self
    }

    pub fn luts(&self) -> &crate::looks::LutChain {
        &self.luts
    }

    pub fn with_pixel_pipeline(mut self, pipeline: crate::pixel_pipeline::PixelPipeline) -> Self {
        self.pixel_pipeline = if self.is_positive() {
            pipeline
        } else {
            Default::default()
        };
        self
    }

    pub(crate) fn pixel_pipeline(&self) -> crate::pixel_pipeline::PixelPipeline {
        self.pixel_pipeline.clone()
    }
}

impl ProcessingInput {
    pub(crate) fn require_source(&self, source: &ProcessingSource) -> Result<(), Error> {
        if let Processing::Parameters(parameters) = &self.processing
            && parameters.graded()
        {
            return Err(Error::InvalidParameter(
                "photographic input requires captured interpretation and grading",
            ));
        }
        if let Processing::Photograph {
            source: program,
            grading,
            area,
        } = &self.processing
        {
            program.require_source(source)?;
            grading.require_geometry(source.geometry)?;
            if let Some(geometry) = source.geometry.filter(|geometry| geometry.is_pair()) {
                for region in geometry.regions() {
                    if region.x < area.x
                        || region.y < area.y
                        || region.x >= area.x + area.width
                        || region.y >= area.y + area.height
                    {
                        return Err(Error::Gpu(
                            "composed input and source geometry regions differ".into(),
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

impl ProcessingInput {
    pub(crate) fn is_positive(&self) -> bool {
        match &self.processing {
            Processing::Parameters(parameters) => parameters.graded(),
            Processing::Photograph { .. } => true,
        }
    }

    pub(crate) fn parameters(&self) -> ProcessingParameters {
        match &self.processing {
            Processing::Parameters(parameters) => **parameters,
            Processing::Photograph {
                source, grading, ..
            } => ProcessingParameters::interpret(source.input, grading.roll, source.matrix)
                .expect("captured photographic constants were admitted"),
        }
    }

    pub(crate) fn grade(&self) -> Option<&Grading> {
        match &self.processing {
            Processing::Photograph { grading, .. } => Some(grading),
            _ => None,
        }
    }

    pub(crate) fn source_transform(&self) -> Option<&InputTransform> {
        match &self.processing {
            Processing::Photograph { source, .. } => Some(source),
            _ => None,
        }
    }

    pub(crate) fn grade_shared(&self) -> Option<&Arc<Grading>> {
        match &self.processing {
            Processing::Photograph { grading, .. } => Some(grading),
            _ => None,
        }
    }
}

impl ProcessingInput {
    pub(crate) fn look_id(&self) -> Option<crate::looks::LookId> {
        self.luts.id()
    }

    /// Look changes rerender, but may reuse an immutable graded AP1 prefix.
    pub(crate) fn same_photographic(&self, other: &Self) -> bool {
        if self.repairs != other.repairs {
            return false;
        }
        self.processing.same_content(&other.processing)
            && self.pixel_pipeline == other.pixel_pipeline
    }
}

impl PartialEq for ProcessingInput {
    fn eq(&self, other: &Self) -> bool {
        self.look_id() == other.look_id() && self.same_photographic(other)
    }
}

impl std::fmt::Debug for ProcessingInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessingInput")
            .field(
                "inspection",
                &matches!(self.processing, Processing::Parameters(_)),
            )
            .field("look", &self.look_id())
            .field("pixel_pipeline", &self.pixel_pipeline)
            .field(
                "snapshot",
                &match &self.processing {
                    Processing::Photograph { grading, .. } => Some(grading.snapshot()),
                    _ => None,
                },
            )
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod halation_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        grain::{GrainPreset, GrainSettings},
        pixel_pipeline::{Grain, PixelPipeline},
    };

    #[cfg(all(
        feature = "project",
        feature = "render",
        any(feature = "raw", feature = "ui")
    ))]
    #[test]
    fn one_recipe_captures_interpretation_roll_layers_crop_seed_and_look() {
        use crate::project::{Development, Project, Source, SourceDevelopment, SourceId};
        let mut project = Project {
            input: crate::processing::InputSettings::Digital,
            ..Default::default()
        };
        let id = project
            .add_source(Source {
                id: SourceId(0),
                path: "authored.dng".into(),
                reference_only: false,
                development: SourceDevelopment {
                    decode: Development::new("test".into(), [1.; 4]),
                    lens_correction: None,
                    dng_development: None,
                    dimensions: [624, 424],
                    active_area: [12, 12, 600, 400],
                    camera_model: "test".into(),
                    camera_to_ap1: DMat3::from_diagonal(DVec3::new(0.9, 1.1, 1.2))
                        .to_cols_array_2d(),
                    input_floor: crate::processing::CAMERA_INPUT_FLOOR,
                },
                orientation: Default::default(),
                crop: Some(crate::crop::Crop {
                    center: [312., 212.],
                    size: [300, 200],
                    angle: 0.19,
                }),
                repairs: Default::default(),
                adjustment_layers: Some(crate::project::AdjustmentLayers::with_global(
                    crate::adjustment_layers::AdjustmentLayerSettings {
                        exposure: 0.2,
                        saturation: 0.8,
                        ..Default::default()
                    },
                )),
            })
            .unwrap();
        project.roll.exposure = 0.35;
        project.output.grain = crate::grain::GrainSettings {
            enabled: true,
            size: 0.04,
            image_sigma: 0.04,
            ..Default::default()
        };
        project.output.lut = Some(Arc::new(
            crate::looks::LookAsset::new(
                "Captured".into(),
                crate::looks::LookOrigin::Imported,
                Arc::new(crate::looks::Lut3D::new(2, [0.; 3], [1.; 3], vec![[0.4; 3]; 8]).unwrap()),
            )
            .unwrap(),
        ));
        let mut recipe = project.image_recipe(id).unwrap();
        recipe.neutral.reference = Some(crate::exposure_reference::ReferenceExposure::Matched(
            crate::exposure_reference::StartingExposure {
                ev: 1.,
                samples: 8,
                spread: 0.,
            },
        ));
        let captured = ProcessingInput::from_recipe(&wgpu::Limits::default(), &recipe).unwrap();
        assert_eq!(
            captured.input_transform().matrix,
            DMat3::from_diagonal(DVec3::new(1.8, 2.2, 2.4))
        );
        assert_ne!(recipe.roll, RollSettings::default());
        assert!(recipe.grade.layers().iter().any(|layer| layer.is_active()));
        assert_eq!(
            captured.input_transform().input,
            recipe.neutral.processing_input().unwrap()
        );
        assert_eq!(captured.grade().unwrap().roll, recipe.roll);
        assert_eq!(captured.snapshot(), &recipe.grade);
        let pipeline = captured.pixel_pipeline();
        let mtf = pipeline
            .stages
            .iter()
            .find_map(|stage| stage.get::<crate::image_mtf::ImageMtfParameters>())
            .unwrap();
        let grain = pipeline
            .stages
            .iter()
            .find_map(|stage| stage.get::<crate::pixel_pipeline::Grain>())
            .unwrap();
        assert!((mtf.sigma - 0.08).abs() < 1e-12);
        assert!((grain.parameters.grain_size - 0.11).abs() < 1e-12);
        assert_eq!(grain.seed, id.0);
        assert_eq!(captured.luts().id(), recipe.luts.id());
        let override_view = captured.clone().with_pixel_pipeline(Default::default());
        assert!(override_view.pixel_pipeline().is_empty());
        assert_eq!(recipe.pixel_pipeline().unwrap(), pipeline);
        assert_ne!(override_view.clone(), captured.clone());
        project.output.grain.enabled = false;
        project.output.lut = None;
        project.source_mut(id).unwrap().crop = None;
        assert_eq!(captured.pixel_pipeline(), pipeline);
        assert!(
            ProcessingInput::from_recipe(
                &wgpu::Limits::default(),
                &project.image_recipe(id).unwrap(),
            )
            .unwrap()
            .pixel_pipeline()
            .is_empty()
        );
    }

    #[test]
    fn source_inspection_cannot_adopt_grain() {
        let source = ProcessingInput::from(ProcessingParameters::source());
        let pipeline =
            PixelPipeline::film(GrainPreset::ThirtyFiveMm.settings(), [3000, 2000], 17).unwrap();
        assert!(!pipeline.is_empty(), "the attempted effect must be active");
        let film = source.clone().with_pixel_pipeline(pipeline);
        assert_eq!(source, film);
        assert!(film.pixel_pipeline().is_empty());
    }

    fn positive_fixture() -> ProcessingInput {
        ProcessingInput::from(
            ProcessingParameters::interpret(
                crate::processing::InputSettings::Digital,
                Default::default(),
                glam::DMat3::IDENTITY,
            )
            .unwrap(),
        )
    }

    #[test]
    fn captured_crop_dimensions_change_emulation_identity_but_not_the_clean_plate() {
        let base = positive_fixture();
        let capture = |film, size| {
            base.clone()
                .with_pixel_pipeline(PixelPipeline::film(film, size, 17).unwrap())
        };
        assert_eq!(
            capture(Default::default(), [6000, 4000]),
            capture(Default::default(), [3000, 2000])
        );
        assert_ne!(
            capture(GrainPreset::ThirtyFiveMm.settings(), [6000, 4000]),
            capture(GrainPreset::ThirtyFiveMm.settings(), [3000, 2000])
        );
    }

    #[cfg(all(feature = "ui", feature = "render"))]
    #[test]
    fn enabled_processing_requires_explicit_crop_context_not_a_source_or_tile_fallback() {
        assert!(PixelPipeline::film(GrainPreset::ThirtyFiveMm.settings(), [0; 2], 17).is_err());
        let pipeline =
            PixelPipeline::film(GrainPreset::ThirtyFiveMm.settings(), [3000, 2000], 17).unwrap();
        let input = positive_fixture().with_pixel_pipeline(pipeline);
        assert!(!input.pixel_pipeline().is_empty());
    }

    #[test]
    fn grain_changes_photographic_content_identity() {
        let base = positive_fixture();
        let settings = GrainSettings {
            enabled: true,
            size: 0.025,
            image_sigma: 0.025,
            amount: 0.040,
            ..Default::default()
        };
        let capture = |seed| {
            base.clone()
                .with_pixel_pipeline(PixelPipeline::film(settings, [3000, 2000], seed).unwrap())
        };
        let film = capture(0);
        assert_ne!(base, film);
        assert!(!base.same_photographic(&film));
        assert!(
            film.pixel_pipeline()
                .stages
                .iter()
                .any(|stage| stage.get::<Grain>().is_some())
        );
        assert_ne!(film, capture(42));
    }
}
