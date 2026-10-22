//! Immutable accepted positive-image choices, independent of execution and output target.
use crate::{
    Error,
    dust_repair::Repair,
    flat_field::FlatField,
    geometry::Composition,
    local_adjustments::PhotoGradeSnapshot,
    processing::{Calibration, PhotoSettings, RollSettings},
    project::{Development, Project, Source, SourceDevelopment, SourceId},
};
use std::sync::Arc;

/// Source identity and upstream interpretation. File freshness is supplied by each source owner.
#[derive(Clone, Debug, PartialEq)]
pub struct NeutralRecipe {
    pub source: SourceId,
    pub development: Development,
    pub source_development: SourceDevelopment,
    pub calibration: Calibration,
    pub inversion: RollSettings,
    pub correction: Option<Arc<FlatField>>,
}

/// Accepted state only: drafts, cache readiness and consumer format do not belong here.
#[derive(Clone, Debug)]
pub struct ImageRecipe {
    pub neutral: NeutralRecipe,
    pub roll: RollSettings,
    pub grade: PhotoGradeSnapshot,
    pub composition: Composition,
    pub look: Option<Arc<crate::looks::LookAsset>>,
    repairs: Vec<Repair>,
}

impl PartialEq for ImageRecipe {
    fn eq(&self, other: &Self) -> bool {
        self.roll == other.roll
            && self.grade == other.grade
            && self.composition == other.composition
            && self.look.as_ref().map(|look| look.lut().id())
                == other.look.as_ref().map(|look| look.lut().id())
            && self.same_neutral_content(other)
    }
}

impl NeutralRecipe {
    pub fn processing_calibration(&self) -> Result<Calibration, Error> {
        self.source_development
            .processing_calibration(self.calibration, self.correction.as_deref())
    }
}

impl ImageRecipe {
    /// Conservative cache charge including referenced correction payload, without deduplicating Arcs.
    pub fn retained_bytes(&self) -> usize {
        let development = &self.neutral.development;
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
            - std::mem::size_of::<PhotoGradeSnapshot>()
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
        let calibration = self
            .calibration()?
            .ok_or_else(|| Error::Project("positive recipe requires calibration".into()))?;
        self.image_recipe_for_source(source, calibration, self.roll, source.photo)
    }

    /// Navigation thumbnails capture grading and geometry without retaining or evaluating dust.
    #[cfg(feature = "editor")]
    pub(crate) fn thumbnail_recipe(&self, id: SourceId) -> Result<ImageRecipe, Error> {
        let source = self.source(id)?;
        let calibration = self
            .calibration()?
            .ok_or_else(|| Error::Project("positive recipe requires calibration".into()))?;
        self.capture_recipe(
            source,
            calibration,
            self.roll,
            source.photo,
            RepairCapture::Omit,
        )
    }

    /// Explicit viewer/candidate overlays reuse recipe construction without changing accepted records.
    pub(crate) fn image_recipe_for_source(
        &self,
        source: &Source,
        calibration: Calibration,
        roll: RollSettings,
        photo: PhotoSettings,
    ) -> Result<ImageRecipe, Error> {
        self.capture_recipe(source, calibration, roll, photo, RepairCapture::Accepted)
    }

    fn capture_recipe(
        &self,
        source: &Source,
        calibration: Calibration,
        roll: RollSettings,
        photo: PhotoSettings,
        repairs: RepairCapture,
    ) -> Result<ImageRecipe, Error> {
        let development = self.development.clone().ok_or_else(|| {
            Error::Project("positive recipe requires development identity".into())
        })?;
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
                development,
                source_development: source.development.clone(),
                calibration,
                inversion: roll.inversion_only(),
                correction: self.active_flat_field().map(|field| field.map.clone()),
            },
            roll,
            grade: PhotoGradeSnapshot::new(photo, source.grade_snapshot()?.locals().to_vec())?,
            composition,
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
