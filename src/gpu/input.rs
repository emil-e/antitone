use super::{GpuInputProgram, Grading, ProcessingParameters, ProcessingSource};
use crate::{
    Error,
    adjustment_layers::AdjustmentLayerSnapshot,
    film_emulation::FilmEmulationSettings,
    geometry::SourceRegion,
    halation::HalationSettings,
    processing::{InputSettings, RollSettings},
};
use glam::{DMat3, DVec3};
use std::sync::Arc;

/// Captured source and photographic settings. Components prepare their own GPU work.
#[derive(Clone)]
pub struct ProcessingInput {
    processing: Processing,
    look: Option<Arc<crate::looks::LookAsset>>,
    film_emulation: FilmEmulationSettings,
    halation: HalationSettings,
    film_seed: Option<u64>,
    pixel_pipeline: Option<crate::pixel_pipeline::PixelPipeline>,
    frame_size: [u32; 2],
    pub(crate) repairs: super::repairs::Snapshot,
}

#[derive(Clone)]
enum Processing {
    Parameters(Arc<ProcessingParameters>),
    Photograph {
        source: Arc<GpuInputProgram>,
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
            frame_size: [0; 2],
            look: None,
            film_emulation: FilmEmulationSettings::default(),
            halation: HalationSettings::default(),
            film_seed: None,
            pixel_pipeline: None,
            repairs: Default::default(),
        }
    }

    pub fn new(
        device: &wgpu::Device,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        layers: AdjustmentLayerSnapshot,
        matrix: DMat3,
        area: SourceRegion,
    ) -> Result<Self, Error> {
        Self::compose(
            device,
            GpuInputProgram::new(input, matrix)?,
            Arc::new(Grading::new(device, roll, layers, area)?),
            area,
        )
    }

    pub fn compose(
        device: &wgpu::Device,
        input: GpuInputProgram,
        grading: Arc<Grading>,
        area: SourceRegion,
    ) -> Result<Self, Error> {
        grading.require_extent(area)?;
        Self::validate(
            device,
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
        super::photograph::prepare(
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
        super::photograph::prepare(
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
        device: &wgpu::Device,
        bound: DVec3,
    ) -> Result<(), Error> {
        if let Processing::Photograph {
            source,
            grading,
            area,
        } = &self.processing
        {
            Self::validate_repaired(
                device,
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

    pub fn input_program(&self) -> &GpuInputProgram {
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
    /// Install the same captured component definitions used by accepted-recipe export.
    /// Interactive overlays choose their own recipe; they do not mutate accepted export intent.
    #[cfg(all(
        feature = "project",
        feature = "render",
        any(feature = "raw", feature = "ui")
    ))]
    pub(crate) fn with_recipe_components(
        self,
        recipe: &crate::image_recipe::ImageRecipe,
    ) -> Result<Self, crate::Error> {
        if !self.parameters().graded() {
            return Err(crate::Error::InvalidParameter(
                "positive recipe requires photographic processing",
            ));
        }
        Ok(self
            .with_look(recipe.look.clone())
            .with_film_emulation(recipe.film_emulation)
            .with_halation(recipe.halation)
            .with_frame_size(recipe.composition.crop().size)
            .with_film_seed(recipe.neutral.source.0)
            .with_pixel_pipeline(recipe.pixel_pipeline()?))
    }

    /// Rendering is downstream of the photographic constants; source inspection never adopts a look.
    pub fn with_look(mut self, look: Option<Arc<crate::looks::LookAsset>>) -> Self {
        self.look = if self.parameters().graded() {
            look
        } else {
            None
        };
        self
    }

    pub fn look(&self) -> Option<&Arc<crate::looks::LookAsset>> {
        self.look.as_ref()
    }

    pub fn with_pixel_pipeline(mut self, pipeline: crate::pixel_pipeline::PixelPipeline) -> Self {
        self.pixel_pipeline = self.parameters().graded().then_some(pipeline);
        self
    }

    pub fn with_film_emulation(mut self, film_emulation: FilmEmulationSettings) -> Self {
        self.pixel_pipeline = None;
        self.film_emulation = if self.parameters().graded() {
            film_emulation
        } else {
            FilmEmulationSettings::default()
        };
        self
    }

    pub fn with_halation(mut self, halation: HalationSettings) -> Self {
        self.pixel_pipeline = None;
        self.halation = if self.parameters().graded() {
            halation
        } else {
            HalationSettings::default()
        };
        self
    }

    pub fn halation(&self) -> HalationSettings {
        self.halation
    }

    /// Capture the photograph's crop dimensions independently of source/tile/view geometry.
    pub fn with_frame_size(mut self, size: [u32; 2]) -> Self {
        self.frame_size = if self.parameters().graded() {
            size
        } else {
            [0; 2]
        };
        self
    }

    pub fn with_film_seed(mut self, seed: u64) -> Self {
        self.film_seed = Some(seed);
        self
    }

    pub fn film_seed(&self) -> Option<u64> {
        self.film_seed
    }

    pub fn film_emulation(&self) -> FilmEmulationSettings {
        self.film_emulation
    }

    pub(crate) fn pixel_pipeline(
        &self,
    ) -> Result<crate::pixel_pipeline::PixelPipeline, crate::Error> {
        if let Some(pipeline) = &self.pixel_pipeline {
            return Ok(pipeline.clone());
        }
        crate::pixel_pipeline::PixelPipeline::emulation(
            self.film_emulation,
            self.halation,
            self.frame_size,
            self.film_seed.unwrap_or(0),
        )
    }

    pub fn signature(&self) -> ProcessingSignature {
        ProcessingSignature {
            processing: self.processing.clone(),
            look: self.look.as_ref().map(|look| look.lut().id()),
            film_emulation: self.film_emulation,
            halation: self.halation.active().then_some(self.halation),
            film_seed: self.film_seed,
            pixel_pipeline: self.pixel_pipeline.clone(),
            frame_size: (self.film_emulation.enabled || self.halation.active())
                .then_some(self.frame_size),
            repairs: self.repairs.clone(),
        }
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

    pub(crate) fn grade(&self) -> Option<&super::Grading> {
        match &self.processing {
            Processing::Photograph { grading, .. } => Some(grading),
            _ => None,
        }
    }

    pub(crate) fn source_program(&self) -> Option<&GpuInputProgram> {
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

/// Content identity excludes GPU allocation identity and document-only adjustment metadata.
#[derive(Clone)]
pub struct ProcessingSignature {
    processing: Processing,
    look: Option<crate::looks::LookId>,
    film_emulation: FilmEmulationSettings,
    halation: Option<HalationSettings>,
    film_seed: Option<u64>,
    frame_size: Option<[u32; 2]>,
    pixel_pipeline: Option<crate::pixel_pipeline::PixelPipeline>,
    repairs: super::repairs::Snapshot,
}

impl ProcessingSignature {
    pub(crate) fn look_id(&self) -> Option<crate::looks::LookId> {
        self.look
    }

    /// Look changes rerender, but may reuse an immutable graded AP1 prefix.
    pub(crate) fn same_photographic(&self, other: &Self) -> bool {
        if self.repairs != other.repairs {
            return false;
        }
        self.processing.same_content(&other.processing)
            && self.film_emulation == other.film_emulation
            && self.halation == other.halation
            && self.film_seed == other.film_seed
            && self.frame_size == other.frame_size
            && self.pixel_pipeline == other.pixel_pipeline
    }
}

impl PartialEq for ProcessingSignature {
    fn eq(&self, other: &Self) -> bool {
        self.look == other.look
            && self.film_emulation == other.film_emulation
            && self.same_photographic(other)
    }
}

impl std::fmt::Debug for ProcessingSignature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessingSignature")
            .field(
                "inspection",
                &matches!(self.processing, Processing::Parameters(_)),
            )
            .field("look", &self.look)
            .field("film_emulation", &self.film_emulation)
            .field("halation", &self.halation)
            .field("film_seed", &self.film_seed)
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
    use crate::film_emulation::FilmPreset;

    #[cfg(all(
        feature = "project",
        feature = "render",
        any(feature = "raw", feature = "ui")
    ))]
    #[test]
    fn accepted_components_capture_crop_seed_and_look_without_adopting_view_overrides() {
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
                    camera_to_ap1: glam::DMat3::IDENTITY.to_cols_array_2d(),
                    input_floor: crate::processing::CAMERA_INPUT_FLOOR,
                },
                orientation: Default::default(),
                crop: Some(crate::crop::Crop {
                    center: [312., 212.],
                    size: [300, 200],
                    angle: 0.19,
                }),
                repairs: Default::default(),
                adjustment_layers: None,
            })
            .unwrap();
        project.output.film_emulation = crate::film_emulation::FilmEmulationSettings {
            enabled: true,
            size: 0.04,
            image_sigma: 0.04,
            ..Default::default()
        };
        project.output.look = Some(Arc::new(
            crate::looks::LookAsset::new(
                "Captured".into(),
                crate::looks::LookOrigin::Imported,
                Arc::new(crate::looks::Lut3D::new(2, [0.; 3], [1.; 3], vec![[0.4; 3]; 8]).unwrap()),
            )
            .unwrap(),
        ));
        let recipe = project.image_recipe(id).unwrap();
        let base = ProcessingInput::from(
            ProcessingParameters::interpret(
                crate::processing::InputSettings::Digital,
                Default::default(),
                glam::DMat3::IDENTITY,
            )
            .unwrap(),
        );
        let captured = base.clone().with_recipe_components(&recipe).unwrap();
        let pipeline = captured.pixel_pipeline().unwrap();
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
        assert_eq!(
            captured.look().unwrap().lut().id(),
            recipe.look.as_ref().unwrap().lut().id()
        );
        let override_view = captured.clone().with_pixel_pipeline(Default::default());
        assert!(override_view.pixel_pipeline().unwrap().is_empty());
        assert_eq!(recipe.pixel_pipeline().unwrap(), pipeline);
        assert_ne!(override_view.signature(), captured.signature());
        project.output.film_emulation.enabled = false;
        project.output.look = None;
        project.source_mut(id).unwrap().crop = None;
        assert_eq!(captured.pixel_pipeline().unwrap(), pipeline);
        assert!(
            base.with_recipe_components(&project.image_recipe(id).unwrap())
                .unwrap()
                .pixel_pipeline()
                .unwrap()
                .is_empty()
        );
        assert!(
            ProcessingInput::from(ProcessingParameters::source())
                .with_recipe_components(&recipe)
                .is_err()
        );
    }

    #[test]
    fn source_inspection_cannot_adopt_film_emulation() {
        let source = ProcessingInput::from(ProcessingParameters::source());
        let film =
            source
                .clone()
                .with_film_emulation(crate::film_emulation::FilmEmulationSettings {
                    enabled: true,
                    size: 0.065,
                    image_sigma: 0.065,
                    amount: 0.065,
                    ..Default::default()
                });
        assert_eq!(source.signature(), film.signature());
        assert!(!film.film_emulation().enabled);
    }

    #[test]
    fn captured_crop_dimensions_change_emulation_identity_but_not_the_clean_plate() {
        let base = ProcessingInput::from(
            ProcessingParameters::interpret(
                crate::processing::InputSettings::Digital,
                Default::default(),
                glam::DMat3::IDENTITY,
            )
            .unwrap(),
        );
        assert_eq!(
            base.clone().with_frame_size([6000, 4000]).signature(),
            base.clone().with_frame_size([3000, 2000]).signature()
        );
        let film = base.with_film_emulation(FilmPreset::ThirtyFiveMm.settings());
        assert_ne!(
            film.clone().with_frame_size([6000, 4000]).signature(),
            film.with_frame_size([3000, 2000]).signature()
        );
    }

    #[cfg(all(feature = "ui", feature = "render"))]
    #[test]
    fn enabled_processing_requires_explicit_crop_context_not_a_source_or_tile_fallback() {
        let input = ProcessingInput::from(
            ProcessingParameters::interpret(
                crate::processing::InputSettings::Digital,
                Default::default(),
                glam::DMat3::IDENTITY,
            )
            .unwrap(),
        )
        .with_film_emulation(FilmPreset::ThirtyFiveMm.settings());
        assert!(input.pixel_pipeline().is_err());
        assert!(input.with_frame_size([3000, 2000]).pixel_pipeline().is_ok());
    }

    #[test]
    fn film_emulation_changes_photographic_content_identity() {
        let base = ProcessingInput::from(
            ProcessingParameters::interpret(
                crate::processing::InputSettings::Digital,
                Default::default(),
                glam::DMat3::IDENTITY,
            )
            .unwrap(),
        );
        let film = base
            .clone()
            .with_film_emulation(crate::film_emulation::FilmEmulationSettings {
                enabled: true,
                size: 0.025,
                image_sigma: 0.025,
                amount: 0.040,
                ..Default::default()
            });
        assert_ne!(base.signature(), film.signature());
        // Graded pixels cannot be reused when the emulation definition changes.
        assert!(!base.signature().same_photographic(&film.signature()));
        assert!(film.film_emulation().enabled);
        assert_ne!(
            film.signature(),
            film.clone().with_film_seed(42).signature()
        );
    }
}
