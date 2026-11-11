use crate::adjustment_layers::AdjustmentLayerSettings;
#[cfg(test)]
use crate::color::Balance;
use crate::{
    Error, PROCESSING_MODEL_VERSION,
    color::wb_matrix,
    crop::Crop,
    geometry::{PhotoOrientation, RollOrientation, SourceRegion},
    processing::{Calibration, RollSettings, exposure_multiplier},
};
use glam::DMat3;
#[cfg(test)]
use glam::DVec3;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::File,
    io::{BufReader, BufWriter, Write},
    path::{Path, PathBuf},
};

mod json;

pub const SCHEMA_VERSION: u32 = 20;
/// Negative scanning is independent of the camera's as-shot white balance.
pub const UNITY_WB: [f64; 4] = [1.; 4];
// Bump when native/raw_bridge.cpp development or scan.rs matrix/normalization/floor contracts change.
pub const OUTPUT_CONTRACT: &str = "ocio-2.6.0-aces2-100nit-antitone-srgb-displayp3-v2";
type Result<T> = std::result::Result<T, Error>;

fn failure(error: impl std::fmt::Display) -> Error {
    Error::Project(error.to_string())
}

fn require(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(failure(message))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SourceId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DevelopmentMode {
    Unity,
    AsShot,
}

impl DevelopmentMode {
    pub fn for_input(input: crate::processing::InputSettings) -> Self {
        if input.is_negative() {
            Self::Unity
        } else {
            Self::AsShot
        }
    }

    pub fn white_balance(self) -> Option<[f64; 4]> {
        match self {
            Self::Unity => Some(UNITY_WB),
            Self::AsShot => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Development {
    pub white_balance: [f64; 4],
    pub backend: Developer,
    #[serde(default)]
    pub backend_version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Developer {
    LibRaw,
    DngSdk,
}

impl Development {
    pub fn new(version: String, white_balance: [f64; 4]) -> Self {
        Self {
            white_balance,
            backend: Developer::LibRaw,
            backend_version: version,
        }
    }

    #[cfg(feature = "dng-sdk")]
    pub fn dng(white_balance: [f64; 4]) -> Self {
        Self {
            white_balance,
            backend: Developer::DngSdk,
            backend_version: crate::dng::INTERPRETATION.into(),
        }
    }

    pub fn uses_sdk(&self) -> bool {
        matches!(self.backend, Developer::DngSdk)
    }

    pub fn version(&self) -> &str {
        &self.backend_version
    }

    #[cfg(feature = "raw")]
    pub fn for_source(source: &SourceDevelopment) -> Self {
        let mut development = source.decode.clone();
        match development.backend {
            Developer::LibRaw => development.backend_version = crate::raw::version(),
            #[cfg(feature = "dng-sdk")]
            Developer::DngSdk => development.backend_version = crate::dng::INTERPRETATION.into(),
            #[cfg(not(feature = "dng-sdk"))]
            Developer::DngSdk => {}
        }
        development
    }

    fn same_processing(&self, other: &Self) -> bool {
        self.backend == other.backend && self.white_balance == other.white_balance
    }

    pub(crate) fn validate(&self) -> Result<()> {
        require(
            self.white_balance
                .iter()
                .all(|gain| (*gain as f32).is_finite() && *gain as f32 > 0.),
            "development white balance must be positive finite f32",
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraBasis {
    pub camera_model: String,
    pub camera_to_ap1: [[f64; 3]; 3],
}

impl CameraBasis {
    pub fn require_source(&self, source: &SourceDevelopment) -> Result<()> {
        require(
            *self == source.camera_basis(),
            "native-camera basis differs; use a separate roll/calibration",
        )
    }

    fn validate(&self) -> Result<()> {
        require(
            !self.camera_model.trim().is_empty(),
            "missing native-camera model identity",
        )?;
        crate::processing::validate_camera_matrix(DMat3::from_cols_array_2d(&self.camera_to_ap1))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceDevelopment {
    /// Which developer produced this source and how; acquisition is per source.
    pub decode: Development,
    pub dimensions: [u32; 2],
    pub active_area: [u32; 4],
    pub camera_model: String,
    pub camera_to_ap1: [[f64; 3]; 3],
}

impl SourceDevelopment {
    /// Captured processing arguments, excluding diagnostic version.
    pub fn same_processing(&self, other: &Self) -> bool {
        self.decode.same_processing(&other.decode)
            && self.dimensions == other.dimensions
            && self.active_area == other.active_area
            && self.camera_model == other.camera_model
            && self.camera_to_ap1 == other.camera_to_ap1
    }

    /// Acquisitions and caches describe the decoder producing pixels now, not saved provenance.
    #[cfg(feature = "raw")]
    pub(crate) fn for_runtime(&self) -> Self {
        let mut source = self.clone();
        source.decode = Development::for_source(self);
        source
    }

    #[cfg(feature = "raw")]
    pub(crate) fn from_metadata(
        metadata: &crate::raw::Metadata,
        white_balance: [f64; 4],
    ) -> Result<Self> {
        let area = metadata.image_area()?;
        Ok(Self {
            decode: metadata.development(white_balance),
            dimensions: metadata.source_dimensions()?,
            active_area: [area.x, area.y, area.width, area.height],
            camera_model: metadata.camera_model(),
            camera_to_ap1: metadata.camera_to_ap1()?.to_cols_array_2d(),
        })
    }

    pub fn camera_basis(&self) -> CameraBasis {
        CameraBasis {
            camera_model: self.camera_model.clone(),
            camera_to_ap1: self.camera_to_ap1,
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        self.decode.validate()?;
        self.camera_basis().validate()?;
        require(
            self.dimensions.iter().all(|v| *v > 0)
                && contains(
                    [0, 0, self.dimensions[0], self.dimensions[1]],
                    self.active_area,
                ),
            "invalid source dimensions/active area",
        )?;
        Ok(())
    }
}

fn contains(area: [u32; 4], region: [u32; 4]) -> bool {
    SourceRegion::from(area).contains_region(SourceRegion::from(region))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub id: SourceId,
    pub path: PathBuf,
    pub development: SourceDevelopment,
    /// Interpretation for which the registered development baseline was recorded.
    pub development_mode: DevelopmentMode,

    pub repairs: crate::dust_repair::RepairSet,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adjustment_layers: Option<AdjustmentLayers>,
    #[serde(default, skip_serializing_if = "PhotoOrientation::is_identity")]
    pub orientation: PhotoOrientation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<Crop>,
}

impl Source {
    fn validate(&self) -> Result<()> {
        require(!self.path.as_os_str().is_empty(), "empty source path")?;
        self.development.validate()?;
        require(
            self.development_mode != DevelopmentMode::Unity
                || self.development.decode.white_balance == UNITY_WB,
            "registered scan development requires unity WB",
        )?;
        if let Some(crop) = self.crop {
            crop.region(self.development.active_area.into())?;
        }

        self.repairs.validate(self.development.active_area.into())?;
        if let Some(layers) = &self.adjustment_layers {
            layers.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TiffTarget {
    DisplayP3,
    Srgb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExrPrecision {
    Half,
    Full,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub contract: String,
    pub tiff: TiffTarget,
    pub exr: ExrPrecision,
    #[serde(
        default,
        skip_serializing_if = "crate::grain::GrainSettings::is_default"
    )]
    pub grain: crate::grain::GrainSettings,
    #[serde(
        default,
        skip_serializing_if = "crate::halation::HalationSettings::is_default"
    )]
    pub halation: crate::halation::HalationSettings,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lut: Option<std::sync::Arc<crate::looks::LookAsset>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub film_and_paper: Option<std::sync::Arc<crate::looks::LookAsset>>,
}

impl Output {
    pub fn luts(&self) -> crate::looks::LutChain {
        crate::looks::LutChain {
            lut: self.lut.clone(),
            film_and_paper: self.film_and_paper.clone(),
        }
    }
}

impl Default for Output {
    fn default() -> Self {
        Self {
            contract: OUTPUT_CONTRACT.into(),
            tiff: TiffTarget::DisplayP3,
            exr: ExrPrecision::Half,
            grain: Default::default(),
            halation: Default::default(),
            lut: None,
            film_and_paper: None,
        }
    }
}
mod flat;
pub use flat::FlatFieldSettings;
mod adjustment_layers;
pub use adjustment_layers::{AdjustmentLayerId, AdjustmentLayerRecord, AdjustmentLayers};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceAnalysis {
    pub source: SourceId,
    pub areas: crate::scan_regions::Areas,
}

/// The acquisition basis shared by scan-based stages (Negative now, Slide later).
/// Digital is per-source and does not require one. The fixed WB is deliberately absent:
/// Negative always develops with [`UNITY_WB`], while Digital uses each source's as-shot WB.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanBasis {
    pub camera: CameraBasis,
}

impl ScanBasis {
    pub fn new(source: &Source) -> Self {
        Self::from_development(&source.development)
    }

    pub fn from_development(development: &SourceDevelopment) -> Self {
        Self {
            camera: development.camera_basis(),
        }
    }

    pub fn require_source(&self, source: &SourceDevelopment) -> Result<()> {
        require(
            source.decode.white_balance == UNITY_WB,
            "Negative development requires unity WB",
        )?;
        require(
            self == &Self::from_development(source),
            "sources do not share one scanning setup; use Digital for mixed sources",
        )
    }

    pub(crate) fn validate(&self) -> Result<()> {
        self.camera.validate()
    }
}

/// Project-level scan acquisition. Stage variants decide how to interpret the developed
/// source; this owns the shared development basis and optional illumination correction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanSetup {
    pub basis: ScanBasis,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flat_field: Option<FlatFieldSettings>,
}

impl ScanSetup {
    #[cfg(test)]
    fn ensure_camera_basis(&mut self, basis: &CameraBasis) {
        self.basis.camera = basis.clone();
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    #[serde(deserialize_with = "deserialize_schema")]
    pub schema: u32,
    pub processing_model: u32,
    pub next_source_id: u64,
    pub sources: Vec<Source>,
    // Filmstrip order, containing every registered photo exactly once.
    pub order: Vec<SourceId>,
    pub roll: RollSettings,
    /// How source samples become a positive (per stage; owns the negative calibration).
    pub input: crate::processing::InputSettings,
    #[serde(default, skip_serializing_if = "RollOrientation::is_identity")]
    pub orientation: RollOrientation,
    pub output: Output,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan: Option<ScanSetup>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis_area: Option<[u32; 4]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub analysis_regions: Vec<SourceAnalysis>,
    /// Resolved source initialization accompanies a write without becoming an accepted edit.
    #[serde(
        default,
        deserialize_with = "crate::exposure_metadata::read_cached",
        skip_serializing_if = "crate::exposure_metadata::empty_or_invalid"
    )]
    pub exposure_metadata: Option<crate::exposure_metadata::Metadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub development_metadata: Option<crate::development_metadata::Metadata>,
}

fn deserialize_schema<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<u32, D::Error> {
    let schema = u32::deserialize(deserializer)?;
    if schema != SCHEMA_VERSION {
        return Err(serde::de::Error::custom("unsupported project schema"));
    }
    Ok(schema)
}

impl Default for Project {
    fn default() -> Self {
        Self {
            schema: SCHEMA_VERSION,
            processing_model: PROCESSING_MODEL_VERSION,
            next_source_id: 1,
            sources: Vec::new(),
            order: Vec::new(),
            input: crate::processing::InputSettings::default(),
            roll: RollSettings::default(),
            orientation: RollOrientation::default(),
            output: Output::default(),
            scan: None,
            analysis_area: None,
            analysis_regions: Vec::new(),
            exposure_metadata: None,
            development_metadata: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Save {
    New,
    Replace,
}

#[cfg(test)]
impl Project {
    // Test ergonomics for reading/writing the active negative interpretation without
    // repeating the enum match at every fixture. Production code uses `InputSettings` directly.

    pub fn dmin(&self) -> Option<[f64; 3]> {
        match self.input {
            crate::processing::InputSettings::Negative { dmin, .. } => dmin.map(|v| v.to_array()),
            crate::processing::InputSettings::Digital => None,
        }
    }

    pub fn set_dmin(&mut self, value: Option<[f64; 3]>) {
        if let crate::processing::InputSettings::Negative { dmin, .. } = self.negative_mut() {
            *dmin = value.map(DVec3::from_array);
        }
    }

    pub fn dmax(&self) -> Option<[f64; 3]> {
        match self.input {
            crate::processing::InputSettings::Negative { dmax, .. } => dmax.map(|v| v.to_array()),
            crate::processing::InputSettings::Digital => None,
        }
    }

    pub fn set_dmax(&mut self, value: Option<[f64; 3]>) {
        if let crate::processing::InputSettings::Negative { dmax, .. } = self.negative_mut() {
            *dmax = value.map(DVec3::from_array);
        }
    }

    pub fn set_balance(&mut self, value: Balance) {
        if let crate::processing::InputSettings::Negative { balance, .. } = self.negative_mut() {
            *balance = value;
        }
    }

    pub fn set_gamma(&mut self, value: f64) {
        if let crate::processing::InputSettings::Negative { gamma, .. } = self.negative_mut() {
            *gamma = value;
        }
    }

    pub fn set_response(&mut self, value: crate::processing::Response) {
        if let crate::processing::InputSettings::Negative { response, .. } = self.negative_mut() {
            *response = value;
        }
    }

    fn negative_mut(&mut self) -> &mut crate::processing::InputSettings {
        if self.input.is_negative() {
            return &mut self.input;
        }
        self.input = crate::processing::InputSettings::Negative {
            dmin: None,
            dmax: None,
            response: crate::processing::Response::film(),
            gamma: crate::processing::DEFAULT_GAMMA,
            balance: crate::color::Balance::default(),
        };
        &mut self.input
    }

    pub fn set_camera_basis(&mut self, value: Option<CameraBasis>) {
        if let Some(basis) = value {
            for source in &mut self.sources {
                source.development.camera_model = basis.camera_model.clone();
                source.development.camera_to_ap1 = basis.camera_to_ap1;
            }
            if let Some(scan) = &mut self.scan {
                scan.ensure_camera_basis(&basis);
            }
        }
    }

    pub fn development(&self) -> Option<&Development> {
        self.sources
            .first()
            .map(|source| &source.development.decode)
    }

    pub fn development_mut(&mut self) -> Option<&mut Development> {
        self.sources
            .first_mut()
            .map(|source| &mut source.development.decode)
    }
}

impl Project {
    pub fn scan_basis(&self) -> Option<&ScanBasis> {
        self.scan.as_ref().map(|scan| &scan.basis)
    }

    /// Negative uses unity WB; Digital resolves the source's as-shot WB at acquisition.
    /// Balance policy for a new source or stage change; existing sources retain their resolved WB.
    pub fn acquisition_wb(&self) -> Option<[f64; 4]> {
        self.input.development_wb()
    }

    pub fn flat_field(&self) -> Option<&FlatFieldSettings> {
        self.scan.as_ref().and_then(|scan| scan.flat_field.as_ref())
    }

    pub fn flat_field_mut(&mut self) -> Option<&mut FlatFieldSettings> {
        self.scan.as_mut().and_then(|scan| scan.flat_field.as_mut())
    }

    pub fn set_flat_field(&mut self, flat_field: Option<FlatFieldSettings>) -> Result<()> {
        if flat_field.is_some() {
            self.ensure_scan_setup();
            require(self.scan.is_some(), "correction requires a scan basis")?;
        }
        if let Some(scan) = &mut self.scan {
            scan.flat_field = flat_field;
        }
        Ok(())
    }

    fn ensure_scan_setup(&mut self) {
        if self.scan.is_none()
            && let Some(first) = self
                .sources
                .iter()
                .find(|source| source.development_mode == DevelopmentMode::Unity)
        {
            self.scan = Some(ScanSetup {
                basis: ScanBasis::new(first),
                flat_field: None,
            });
        }
    }
    pub fn source(&self, id: SourceId) -> Result<&Source> {
        self.sources
            .iter()
            .find(|source| source.id == id)
            .ok_or_else(|| failure("unknown source ID"))
    }

    pub fn source_mut(&mut self, id: SourceId) -> Result<&mut Source> {
        self.sources
            .iter_mut()
            .find(|source| source.id == id)
            .ok_or_else(|| failure("unknown source ID"))
    }

    pub fn add_source(&mut self, mut source: Source) -> Result<SourceId> {
        source.development_mode = DevelopmentMode::for_input(self.input);
        source.validate()?;
        // Scan-based stages need one shared scanning setup; assert it at admission so the
        // project cannot accumulate sources the active stage cannot interpret.
        if self.input.requires_scan_sources() {
            self.ensure_scan_setup();
            if let Some(scan) = &self.scan {
                scan.basis.require_source(&source.development)?;
            } else {
                self.scan = Some(ScanSetup {
                    basis: ScanBasis::new(&source),
                    flat_field: None,
                });
            }
        }
        let id = SourceId(self.next_source_id);
        let next = self
            .next_source_id
            .checked_add(1)
            .ok_or_else(|| failure("source ID space exhausted"))?;
        source.id = id;
        self.sources.push(source);
        self.order.push(id);
        self.next_source_id = next;
        Ok(id)
    }

    /// Assert the shared scanning setup required by scan-based input stages.
    pub fn require_scan_sources(&self) -> Result<()> {
        let active = &self.order;
        let Some(basis) = self
            .scan
            .as_ref()
            .map(|scan| scan.basis.clone())
            .or_else(|| {
                active
                    .first()
                    .and_then(|id| self.source(*id).ok())
                    .map(ScanBasis::new)
            })
        else {
            return Ok(());
        };
        for id in &self.order {
            basis.require_source(&self.source(*id)?.development)?;
        }
        Ok(())
    }

    pub fn require_scan_source(&self, source: &SourceDevelopment) -> Result<()> {
        self.scan
            .as_ref()
            .map(|scan| scan.basis.clone())
            .or_else(|| {
                self.order
                    .iter()
                    .filter_map(|id| self.source(*id).ok())
                    .find(|source| source.development_mode == DevelopmentMode::Unity)
                    .map(ScanBasis::new)
            })
            .map_or(Ok(()), |basis| basis.require_source(source))
    }

    /// The shared acquisition basis of the roll's sources, when there are any.
    pub fn camera_basis(&self) -> Option<CameraBasis> {
        self.scan
            .as_ref()
            .map(|scan| scan.basis.camera.clone())
            .or_else(|| {
                self.sources
                    .first()
                    .map(|source| source.development.camera_basis())
            })
    }

    #[cfg(test)]
    pub(crate) fn reset_scan_basis(&mut self) {
        let Some(first) = self
            .order
            .first()
            .and_then(|id| self.source(*id).ok())
            .or_else(|| self.sources.first())
        else {
            return;
        };
        let basis = ScanBasis::new(first);
        match &mut self.scan {
            Some(scan) => scan.basis = basis,
            None => {
                self.scan = Some(ScanSetup {
                    basis,
                    flat_field: None,
                });
            }
        }
    }

    /// Delete the photo and its analysis support; correction provenance is independent.
    pub fn remove_from_filmstrip(&mut self, id: SourceId) -> Result<()> {
        self.source(id)?;
        self.order.retain(|candidate| *candidate != id);
        self.sources.retain(|source| source.id != id);
        self.analysis_regions.retain(|entry| entry.source != id);
        Ok(())
    }

    pub fn resolve_path(&self, id: SourceId, project_path: &Path) -> Result<PathBuf> {
        let path = &self.source(id)?.path;
        Ok(if path.is_absolute() {
            path.clone()
        } else {
            project_path.parent().unwrap_or(Path::new(".")).join(path)
        })
    }

    pub fn calibration(&self) -> Result<Option<Calibration>> {
        self.input
            .calibration()
            .map(|calibration| calibration.validate())
            .transpose()
    }

    pub fn active_flat_field(&self) -> Option<&FlatFieldSettings> {
        self.flat_field()
            .filter(|field| self.input.capabilities().flat_field && field.enabled)
    }

    pub fn validate(&self) -> Result<()> {
        require(self.schema == SCHEMA_VERSION, "unsupported project schema")?;
        require(
            self.processing_model == PROCESSING_MODEL_VERSION,
            "unsupported processing model; no automatic migration",
        )?;
        if let Some(negative) = self.input.negative_input() {
            negative.channel_gammas()?;
        }
        self.output.grain.validate()?;
        self.output.halation.validate()?;
        require(
            self.output
                .lut
                .as_ref()
                .is_none_or(|asset| matches!(asset.origin(), crate::looks::LookOrigin::Imported)),
            "LUT slot requires an imported asset",
        )?;
        require(
            self.output.film_and_paper.as_ref().is_none_or(|asset| {
                matches!(
                    asset.origin(),
                    crate::looks::LookOrigin::Photochemical { .. }
                )
            }),
            "Film & paper slot requires material provenance",
        )?;
        wb_matrix(self.roll.wb)?;
        exposure_multiplier(self.roll.exposure)?;
        require(
            !self.output.contract.is_empty(),
            "missing output transform identity",
        )?;
        let mut ids = HashSet::new();
        for source in &self.sources {
            require(
                source.id.0 > 0 && source.id.0 < self.next_source_id && ids.insert(source.id),
                "duplicate/invalid source ID allocator",
            )?;
            source.validate()?;
        }
        if let Some(metadata) = &self.development_metadata {
            metadata.validate(self)?;
        }
        let mut ordered = HashSet::new();
        for id in &self.order {
            require(
                ids.contains(id) && ordered.insert(*id),
                "invalid/duplicate ordered source ID",
            )?;
        }
        require(
            ordered.len() == ids.len(),
            "source is absent from filmstrip",
        )?;
        if let Some(region) = self.analysis_area {
            require(
                region[2] >= 8 && region[3] >= 8,
                "analysis area is smaller than a capture footprint",
            )?;
            for id in &self.order {
                require(
                    contains(self.source(*id)?.development.active_area, region),
                    "analysis area is outside a roll source",
                )?;
            }
        }
        let mut analysis_ids = HashSet::new();
        for entry in &self.analysis_regions {
            require(
                analysis_ids.insert(entry.source),
                "duplicate source analysis support",
            )?;
            entry
                .areas
                .validate(self.source(entry.source)?.development.active_area.into())?;
            require(entry.areas.ready(), "incomplete accepted analysis support")?;
        }
        require(self.next_source_id > 0, "invalid source ID allocator")?;
        if let crate::processing::InputSettings::Negative { dmin, dmax, .. } = &self.input {
            for reference in [*dmin, *dmax].into_iter().flatten() {
                require(
                    reference
                        .to_array()
                        .iter()
                        .all(|v| v.is_finite() && *v > 0.),
                    "invalid stored reference RGB",
                )?;
            }
        }
        if let Some(negative) = self.input.negative_input() {
            negative.parameters()?;
        }
        if let Some(scan) = &self.scan {
            scan.basis.validate()?;
            if let Some(field) = &scan.flat_field {
                field.validate()?;
            }
        }
        Ok(())
    }

    pub fn require_runtime(&self) -> Result<()> {
        self.validate()?;
        require(
            self.output.contract == OUTPUT_CONTRACT,
            "output transform contract differs from this runtime",
        )
    }

    pub fn load(path: &Path) -> Result<Self> {
        let input = BufReader::new(json::bounded_strings(File::open(path).map_err(failure)?));
        let project: Self = serde_json::from_reader(input).map_err(failure)?;
        project.validate()?;
        Ok(project)
    }

    pub fn save(&self, path: &Path, policy: Save) -> Result<()> {
        self.write(
            path,
            policy,
            #[cfg(all(test, feature = "editor"))]
            None,
        )
    }

    #[cfg(all(test, feature = "editor"))]
    pub(crate) fn save_observed(
        &self,
        path: &Path,
        policy: Save,
        observer: &WriteObserver,
    ) -> Result<()> {
        self.write(path, policy, Some(observer))
    }

    fn write(
        &self,
        path: &Path,
        policy: Save,
        #[cfg(all(test, feature = "editor"))] observer: Option<&WriteObserver>,
    ) -> Result<()> {
        self.validate()?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(failure)?;
        {
            let mut output = BufWriter::new(&mut temporary);
            serde_json::to_writer_pretty(&mut output, self).map_err(failure)?;
            output.write_all(b"\n").map_err(failure)?;
            output.flush().map_err(failure)?;
        }
        temporary.as_file().sync_all().map_err(failure)?;
        #[cfg(all(test, feature = "editor"))]
        if let Some(observer) = observer {
            observer(WritePhase::TemporarySynced);
        }
        match policy {
            Save::New => install_new(temporary, path)?,
            Save::Replace => {
                temporary.persist(path).map_err(failure)?;
            }
        }
        #[cfg(all(test, feature = "editor"))]
        if let Some(observer) = observer {
            observer(WritePhase::Published);
        }
        Ok(())
    }
}

#[cfg(all(test, feature = "editor"))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum WritePhase {
    TemporarySynced,
    Published,
}

#[cfg(all(test, feature = "editor"))]
pub(crate) type WriteObserver = dyn Fn(WritePhase) + Send + Sync;

/// Installs the completed temporary as a new project, never overwriting an
/// existing file.
///
/// The preferred publication is a no-replace rename, but filesystems such as
/// exFAT — the usual SD-card format — implement neither that rename flag nor
/// the hard links tempfile falls back to, answering `ENOTSUP` instead. Copying
/// into a `create_new` destination keeps the no-overwrite guarantee there,
/// trading away the crash atomicity of a rename that those volumes lack
/// anyway. The `ENOTSUP` answer is the macOS one; on Linux, linkless volumes
/// fail tempfile's hard-link fallback with `EPERM`, which is deliberately
/// not matched because that errno also reports legitimate refusals such as
/// sticky directories.
fn install_new(temporary: tempfile::NamedTempFile, path: &Path) -> Result<()> {
    match temporary.persist_noclobber(path) {
        Ok(_) => Ok(()),
        Err(error) if publication_unsupported(&error.error) => copy_exclusive(&error.file, path),
        Err(error) => Err(failure(error)),
    }
}

/// Whether `error` reports that the filesystem cannot publish a file without
/// replacing an existing name. Everything else — in particular `EEXIST`, the
/// answer for an existing destination — must fail the save as-is.
fn publication_unsupported(error: &std::io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::ENOTSUP) | Some(libc::EOPNOTSUPP)
    )
}

/// Copies the completed, synced `temporary` into a destination created
/// exclusively, so an existing file is never opened for writing.
fn copy_exclusive(temporary: &tempfile::NamedTempFile, path: &Path) -> Result<()> {
    let mut source = temporary.reopen().map_err(failure)?;
    let mut options = File::options();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Match the 0o600 the temporary was created with, so publication via
        // copy yields the same private mode as publication via rename.
        // Filesystems that store no Unix permissions ignore the request.
        options.mode(0o600);
    }
    let mut destination = options.open(path).map_err(failure)?;
    let published =
        std::io::copy(&mut source, &mut destination).and_then(|_| destination.sync_all());
    if let Err(error) = published {
        // A failed new save leaves no file at the destination: existing files
        // were never opened, and a partial copy is removed best-effort.
        let _ = std::fs::remove_file(path);
        return Err(failure(error));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
