//! Captured photographic intent, independent of execution quality and output target.
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

pub(crate) mod quality;

#[derive(Clone, Debug, PartialEq)]
pub struct NeutralRecipe {
    pub source: SourceId,
    pub source_development: SourceDevelopment,
    pub input: InputSettings,
    pub reference: Option<crate::exposure_reference::ReferenceExposure>,
    pub correction: Option<Arc<FlatField>>,
}

/// Complete photographic intent. Owners construct a value before sharing its snapshot;
/// graph operations capture their arguments rather than following subsequent edits.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageRecipe {
    pub neutral: NeutralRecipe,
    pub roll: RollSettings,
    pub grade: AdjustmentLayerSnapshot,
    pub composition: Composition,
    pub grain: crate::grain::GrainSettings,
    pub halation: crate::halation::HalationSettings,
    pub luts: crate::looks::LutChain,
    pub repairs: Vec<Repair>,
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
            + self.luts.retained_bytes()
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
}

impl Project {
    /// Capture accepted intent without acquiring pixels or resolving repairs.
    pub fn image_recipe(&self, id: SourceId) -> Result<ImageRecipe, Error> {
        self.capture_image(self.source(id)?, |_| {})
    }

    /// The owning entry point supplies effective edits inside capture. Only the completed
    /// value leaves this operation; publication uses ordinary shared ownership.
    pub(crate) fn capture_image(
        &self,
        source: &Source,
        effective: impl FnOnce(&mut ImageRecipe),
    ) -> Result<ImageRecipe, Error> {
        source
            .repairs
            .validate(source.development.active_area.into())?;
        let mut recipe = ImageRecipe {
            neutral: NeutralRecipe {
                source: source.id,
                source_development: source.development.clone(),
                input: self.input,
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
            roll: self.roll,
            grade: source.grade_snapshot()?,
            composition: Composition::with_crop(
                source.development.active_area.into(),
                self.orientation,
                source.orientation,
                source.crop,
            )?,
            grain: self.output.grain,
            halation: self.output.halation,
            luts: self.output.luts(),
            repairs: source
                .repairs
                .repairs()
                .iter()
                .filter(|repair| repair.enabled)
                .cloned()
                .collect(),
        };
        // Effective instructions can still await a valid donor or evaluated pixels.
        // Their accepted source records remain validated and unchanged.
        effective(&mut recipe);
        Ok(recipe)
    }
}
