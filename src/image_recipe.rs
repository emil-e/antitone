//! Immutable accepted positive-image choices, independent of execution and output target.
use crate::{
    Error,
    adjustment_layers::AdjustmentLayerSnapshot,
    dust_repair::Repair,
    flat_field::FlatField,
    geometry::Composition,
    processing::{InputSettings, RollSettings},
    project::{Project, Source, SourceDevelopment, SourceId},
};
use std::sync::Arc;

/// Source identity and upstream interpretation. File freshness is supplied by each source owner.
#[derive(Clone, Debug)]
pub struct NeutralRecipe {
    pub source: SourceId,
    pub source_development: SourceDevelopment,
    pub input: InputSettings,
    pub reference: Option<crate::exposure_reference::ReferenceExposure>,
    pub correction: Option<Arc<FlatField>>,
}

impl PartialEq for NeutralRecipe {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && self.source_development == other.source_development
            && self.input == other.input
            && self.correction == other.correction
            // Estimator diagnostics are persisted, but only the gain affects pixels.
            && (self.input.is_negative()
                || self.reference.as_ref().map(|value| value.ev())
                    == other.reference.as_ref().map(|value| value.ev()))
    }
}

/// Accepted state only: drafts, cache readiness and consumer format do not belong here.
#[derive(Clone, Debug)]
pub struct ImageRecipe {
    pub neutral: NeutralRecipe,
    pub roll: RollSettings,
    pub grade: AdjustmentLayerSnapshot,
    pub composition: Composition,
    pub film_emulation: crate::film_emulation::FilmEmulationSettings,
    pub look: Option<Arc<crate::looks::LookAsset>>,
    repairs: Vec<Repair>,
}

impl PartialEq for ImageRecipe {
    fn eq(&self, other: &Self) -> bool {
        self.roll == other.roll
            && self.grade == other.grade
            && self.composition == other.composition
            && self.film_emulation == other.film_emulation
            && self.look.as_ref().map(|look| look.lut().id())
                == other.look.as_ref().map(|look| look.lut().id())
            && self.same_neutral_content(other)
    }
}

impl NeutralRecipe {
    pub fn processing_input(&self) -> Result<InputSettings, Error> {
        match self.input.calibration() {
            Some(base) => Ok(self.input.with_calibration(
                self.source_development
                    .processing_calibration(base, self.correction.as_deref())?,
            )),
            None => Ok(self.input),
        }
    }
}

impl ImageRecipe {
    /// Conservative cache charge including referenced correction payload, without deduplicating Arcs.
    pub fn retained_bytes(&self) -> usize {
        let development = &self.neutral.source_development.decode;
        std::mem::size_of::<Self>()
            + development.backend_version.capacity()
            + development.contract.capacity()
            + development.inversion_space.capacity()
            + development.working_space.capacity()
            + self.neutral.source_development.camera_model.capacity()
            + self
                .neutral
                .source_development
                .lens_correction
                .as_ref()
                .map_or(0, String::capacity)
            + self
                .neutral
                .correction
                .as_ref()
                .map_or(0, |field| field.retained_bytes())
            + self.look.as_ref().map_or(0, |look| look.retained_bytes())
            + self.grade.retained_bytes()
            - std::mem::size_of::<AdjustmentLayerSnapshot>()
            + self.repairs.capacity() * std::mem::size_of::<Repair>()
            + self
                .repairs
                .iter()
                .map(|repair| {
                    repair.support.retained_bytes()
                        - std::mem::size_of::<crate::dust_repair::SupportMask>()
                })
                .sum::<usize>()
    }

    pub fn same_neutral_content(&self, other: &Self) -> bool {
        self.neutral == other.neutral
            && self.repairs.len() == other.repairs.len()
            && self
                .repairs
                .iter()
                .zip(&other.repairs)
                .all(|(a, b)| a.support == b.support && a.donor == b.donor)
    }

    pub fn pixel_pipeline(&self) -> Result<crate::pixel_pipeline::PixelPipeline, Error> {
        crate::pixel_pipeline::PixelPipeline::film(
            self.film_emulation,
            self.composition.crop().size,
            self.neutral.source.0,
        )
    }

    pub fn repairs(&self) -> &[Repair] {
        &self.repairs
    }
}

enum RepairCapture {
    Accepted,
    #[cfg(feature = "editor")]
    Omit,
}

impl Project {
    /// Capture one source's accepted photograph without acquiring pixels or resolving repairs.
    pub fn image_recipe(&self, id: SourceId) -> Result<ImageRecipe, Error> {
        let source = self.source(id)?;
        if self.input.is_negative() && self.input.calibration().is_none() {
            return Err(Error::Project(
                "positive recipe requires calibration".into(),
            ));
        }
        self.image_recipe_for_source(source, &self.input, self.roll)
    }

    /// Navigation thumbnails capture grading and geometry without retaining or evaluating dust.
    #[cfg(feature = "editor")]
    pub(crate) fn thumbnail_recipe(&self, id: SourceId) -> Result<ImageRecipe, Error> {
        let source = self.source(id)?;
        if !self.input.ready() {
            return Err(Error::Project(
                "positive recipe requires calibration".into(),
            ));
        }
        self.capture_recipe(source, &self.input, self.roll, RepairCapture::Omit)
    }

    /// Explicit viewer/candidate overlays reuse recipe construction without changing accepted records.
    pub(crate) fn image_recipe_for_source(
        &self,
        source: &Source,
        input: &InputSettings,
        roll: RollSettings,
    ) -> Result<ImageRecipe, Error> {
        self.capture_recipe(source, input, roll, RepairCapture::Accepted)
    }

    fn capture_recipe(
        &self,
        source: &Source,
        input: &InputSettings,
        roll: RollSettings,

        repairs: RepairCapture,
    ) -> Result<ImageRecipe, Error> {
        let include_repairs = matches!(repairs, RepairCapture::Accepted);
        if include_repairs {
            source
                .repairs
                .validate(source.development.active_area.into())?;
        }
        let composition = Composition::with_crop(
            source.development.active_area.into(),
            self.orientation,
            source.orientation,
            source.crop,
        )?;
        Ok(ImageRecipe {
            neutral: NeutralRecipe {
                source: source.id,
                source_development: source.development.clone(),
                input: *input,
                reference: self
                    .exposure_metadata
                    .as_ref()
                    .filter(|metadata| metadata.valid())
                    .and_then(|metadata| {
                        metadata
                            .entries
                            .iter()
                            .find(|entry| entry.source == source.id)
                    })
                    .map(|entry| entry.reference.clone()),
                correction: self.active_flat_field().map(|field| field.map.clone()),
            },
            roll,
            grade: source.grade_snapshot()?,
            composition,
            film_emulation: self.output.film_emulation,
            look: self.output.look.clone(),
            repairs: if include_repairs {
                source
                    .repairs
                    .repairs()
                    .iter()
                    .filter(|repair| repair.enabled)
                    .cloned()
                    .collect()
            } else {
                Vec::new()
            },
        })
    }
}
