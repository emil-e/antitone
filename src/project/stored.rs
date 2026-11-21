//! Persisted records establish data integrity; execution admission belongs to consumers.
use super::*;
use crate::{color::wb_matrix, processing::exposure_multiplier};

// Format identifiers have no editable runtime value; parsing admits only this contract.
macro_rules! fixed_field {
    ($kind:ty, $wire:ty, $value:expr, $message:literal) => {
        impl Serialize for $kind {
            fn serialize<S: serde::Serializer>(
                &self,
                serializer: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                $value.serialize(serializer)
            }
        }

        impl<'de> Deserialize<'de> for $kind {
            fn deserialize<D: serde::Deserializer<'de>>(
                deserializer: D,
            ) -> std::result::Result<Self, D::Error> {
                let stored = <$wire>::deserialize(deserializer)?;
                if stored != $value {
                    return Err(serde::de::Error::custom($message));
                }
                Ok(Self)
            }
        }
    };
}

fixed_field!(
    SchemaVersion,
    u32,
    SCHEMA_VERSION,
    "unsupported project schema"
);
fixed_field!(
    ProcessingModel,
    u32,
    PROCESSING_MODEL_VERSION,
    "unsupported processing model; no automatic migration"
);
fixed_field!(
    OutputContract,
    String,
    OUTPUT_CONTRACT,
    "output transform contract differs from this runtime"
);

#[derive(Serialize, Deserialize)]
#[serde(remote = "Source", deny_unknown_fields)]
struct SourceRecord {
    id: SourceId,
    path: PathBuf,
    development: SourceDevelopment,
    development_mode: DevelopmentMode,

    repairs: crate::dust_repair::RepairSet,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    adjustment_layers: Option<AdjustmentLayers>,
    #[serde(default, skip_serializing_if = "PhotoOrientation::is_identity")]
    orientation: PhotoOrientation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    crop: Option<Crop>,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "Output", deny_unknown_fields)]
struct OutputRecord {
    contract: OutputContract,
    tiff: TiffTarget,
    exr: ExrPrecision,
    #[serde(
        default,
        skip_serializing_if = "crate::grain::GrainSettings::is_default"
    )]
    grain: crate::grain::GrainSettings,
    #[serde(
        default,
        skip_serializing_if = "crate::halation::HalationSettings::is_default"
    )]
    halation: crate::halation::HalationSettings,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lut: Option<std::sync::Arc<crate::looks::LookAsset>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    film_and_paper: Option<std::sync::Arc<crate::looks::LookAsset>>,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "Project", deny_unknown_fields)]
struct ProjectRecord {
    schema: SchemaVersion,
    processing_model: ProcessingModel,
    next_source_id: u64,
    sources: Vec<Source>,
    order: Vec<SourceId>,
    #[serde(with = "roll")]
    roll: RollSettings,
    #[serde(with = "input")]
    input: crate::processing::InputSettings,
    #[serde(default, skip_serializing_if = "RollOrientation::is_identity")]
    orientation: RollOrientation,
    output: Output,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    flat_field: Option<FlatFieldSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    analysis_area: Option<[u32; 4]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    analysis_regions: Vec<SourceAnalysis>,
    #[serde(
        default,
        deserialize_with = "crate::exposure_metadata::read_cached",
        skip_serializing_if = "crate::exposure_metadata::empty_or_invalid"
    )]
    exposure_metadata: Option<crate::exposure_metadata::Metadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    development_metadata: Option<crate::development_metadata::Metadata>,
}

impl Serialize for Source {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        Source::validate(self).map_err(serde::ser::Error::custom)?;
        SourceRecord::serialize(self, serializer)
    }
}

impl<'de> Deserialize<'de> for Source {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = SourceRecord::deserialize(deserializer)?;
        Source::validate(&value).map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}

impl Serialize for Output {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        self.grain.validate().map_err(serde::ser::Error::custom)?;
        self.halation
            .validate()
            .map_err(serde::ser::Error::custom)?;
        check_asset_roles(self).map_err(serde::ser::Error::custom)?;
        OutputRecord::serialize(self, serializer)
    }
}

impl<'de> Deserialize<'de> for Output {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = OutputRecord::deserialize(deserializer)?;
        check_asset_roles(&value).map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}

impl Serialize for Project {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        check_relations(self).map_err(serde::ser::Error::custom)?;
        ProjectRecord::serialize(self, serializer)
    }
}

impl<'de> Deserialize<'de> for Project {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = ProjectRecord::deserialize(deserializer)?;
        check_relations(&value).map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}

fn check_relations(project: &Project) -> Result<()> {
    let mut ids = HashSet::new();
    for source in &project.sources {
        require(
            source.id.0 > 0 && source.id.0 < project.next_source_id && ids.insert(source.id),
            "duplicate/invalid source ID allocator",
        )?;
    }
    if let Some(metadata) = &project.development_metadata {
        metadata.validate(project)?;
    }
    let mut ordered = HashSet::new();
    for id in &project.order {
        require(
            ids.contains(id) && ordered.insert(*id),
            "invalid/duplicate ordered source ID",
        )?;
    }
    require(
        ordered.len() == ids.len(),
        "source is absent from filmstrip",
    )?;
    if let Some(region) = project.analysis_area {
        require(
            region[2] >= 8 && region[3] >= 8,
            "analysis area is smaller than a capture footprint",
        )?;
        SourceRegion::from(region).validate()?;
    }
    let mut analysis_ids = HashSet::new();
    for entry in &project.analysis_regions {
        require(
            analysis_ids.insert(entry.source),
            "duplicate source analysis support",
        )?;
        entry
            .areas
            .validate(project.source(entry.source)?.development.active_area.into())?;
        require(entry.areas.ready(), "incomplete accepted analysis support")?;
    }
    require(project.next_source_id > 0, "invalid source ID allocator")?;
    Ok(())
}

fn check_asset_roles(output: &Output) -> Result<()> {
    require(
        output
            .lut
            .as_ref()
            .is_none_or(|asset| matches!(asset.origin(), crate::looks::LookOrigin::Imported)),
        "LUT slot requires an imported asset",
    )?;
    require(
        output.film_and_paper.as_ref().is_none_or(|asset| {
            matches!(
                asset.origin(),
                crate::looks::LookOrigin::Photochemical { .. }
                    | crate::looks::LookOrigin::Renderer { .. }
            )
        }),
        "Film & paper slot requires material provenance",
    )
}

mod input {
    use super::*;
    use crate::processing::InputSettings;

    fn check(input: &InputSettings) -> Result<()> {
        input.validate_controls()?;
        if let InputSettings::Negative { dmin, dmax, .. } = input {
            for reference in [*dmin, *dmax].into_iter().flatten() {
                require(
                    reference
                        .to_array()
                        .iter()
                        .all(|value| value.is_finite() && *value > 0.),
                    "invalid stored reference RGB",
                )?;
            }
        }
        Ok(())
    }

    pub fn serialize<S: serde::Serializer>(
        input: &InputSettings,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        check(input).map_err(serde::ser::Error::custom)?;
        input.serialize(serializer)
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<InputSettings, D::Error> {
        let input = InputSettings::deserialize(deserializer)?;
        check(&input).map_err(serde::de::Error::custom)?;
        Ok(input)
    }
}

mod roll {
    use super::*;

    fn check(roll: &RollSettings) -> Result<()> {
        wb_matrix(roll.wb)?;
        exposure_multiplier(roll.exposure)?;
        Ok(())
    }

    pub fn serialize<S: serde::Serializer>(
        roll: &RollSettings,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        check(roll).map_err(serde::ser::Error::custom)?;
        roll.serialize(serializer)
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<RollSettings, D::Error> {
        let roll = RollSettings::deserialize(deserializer)?;
        check(&roll).map_err(serde::de::Error::custom)?;
        Ok(roll)
    }
}
