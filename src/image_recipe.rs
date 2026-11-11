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
    pub source_development: crate::development_metadata::Development,
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
    #[cfg(feature = "raw")]
    /// The consumer owns the immutable source lease and freshness fence, not the selected viewer.
    pub fn require_scan(&self, scan: &crate::scan::Scan) -> Result<(), Error> {
        scan.source().require_input(&self.input)?;
        let actual = SourceDevelopment::from_metadata(
            &scan.source().metadata(),
            scan.source().white_balance(),
        )?;
        if !self.source_development.resolved()?.same_processing(&actual)
            || self.correction.as_ref() != scan.flat_field()
        {
            return Err(Error::Raw(
                "captured recipe differs from developed source".into(),
            ));
        }
        if !self.input.is_negative()
            && let Some(reference) = &self.reference
            && scan.reference_exposure.as_ref().map(|value| value.ev()) != Some(reference.ev())
        {
            return Err(Error::Raw(
                "captured recipe differs from recorded exposure baseline".into(),
            ));
        }
        Ok(())
    }

    pub fn processing_transform(&self) -> Result<(InputSettings, glam::DMat3), Error> {
        let matrix = self.input.positive_matrix(
            glam::DMat3::from_cols_array_2d(&self.source_development.resolved()?.camera_to_ap1),
            self.reference.as_ref().map(|reference| reference.ev()),
        )?;
        Ok((self.input, matrix))
    }
}

impl ImageRecipe {
    /// Conservative cache charge including referenced correction payload, without deduplicating Arcs.
    pub fn retained_bytes(&self) -> usize {
        let registered = self.neutral.source_development.registered();
        let development = &registered.decode;
        std::mem::size_of::<Self>()
            + development.version().len()
            + registered.camera_model.capacity()
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
        let reference = self
            .exposure_metadata
            .as_ref()
            .filter(|metadata| metadata.valid())
            .and_then(|metadata| metadata.entries.iter().find(|entry| entry.source == id))
            .map(|entry| entry.reference.clone());
        self.image_recipe_with_reference(self.source(id)?, reference)
    }

    pub(crate) fn image_recipe_with_reference(
        &self,
        source: &Source,
        reference: Option<crate::exposure_reference::ReferenceExposure>,
    ) -> Result<ImageRecipe, Error> {
        source
            .repairs
            .validate(source.development.active_area.into())?;
        Ok(ImageRecipe {
            neutral: NeutralRecipe {
                source: source.id,
                source_development: crate::development_metadata::Development::captured(
                    source,
                    &source.path,
                    self.input,
                    self.development_metadata
                        .as_ref()
                        .unwrap_or(&Default::default()),
                ),
                input: self.input,
                reference,
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
        })
    }
}
