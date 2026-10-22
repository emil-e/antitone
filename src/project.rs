use crate::{
    Error, PROCESSING_MODEL_VERSION,
    color::wb_matrix,
    crop::Crop,
    geometry::{PhotoOrientation, RollOrientation, SourceRegion},
    processing::{Calibration, Inversion, PhotoSettings, RollSettings, exposure_multiplier},
};
use glam::{DMat3, DVec3};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::File,
    io::{BufReader, BufWriter, Write},
    path::{Path, PathBuf},
};

#[path = "project_json.rs"]
mod json;

pub const SCHEMA_VERSION: u32 = 6;
// Bump when native/raw_bridge.cpp development or scan.rs matrix/normalization/floor contracts change.
pub const DEVELOPMENT_CONTRACT: &str =
    "libraw-camera-ahd-sony-shading-active-bilinear-f32-inversion-ap1-floor-v4";
pub const DNG_DEVELOPMENT_CONTRACT: &str = "adobe-dng-sdk-camera-post-op-wb-native-quantum-v2";
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Development {
    pub backend_version: String,
    pub contract: String,
    pub fixed_wb: [f64; 4],
    pub inversion_space: String,
    pub working_space: String,
}

impl Development {
    pub fn new(backend_version: String, fixed_wb: [f64; 4]) -> Self {
        Self {
            backend_version,
            fixed_wb,
            contract: DEVELOPMENT_CONTRACT.into(),
            inversion_space: "native-camera-RGB".into(),
            working_space: "AP1/D60".into(),
        }
    }

    #[cfg(feature = "dng-sdk")]
    pub fn dng(fixed_wb: [f64; 4]) -> Self {
        Self {
            backend_version: crate::dng::INTERPRETATION.into(),
            contract: DNG_DEVELOPMENT_CONTRACT.into(),
            ..Self::new(String::new(), fixed_wb)
        }
    }

    #[cfg(feature = "raw")]
    pub fn for_source(source: &SourceDevelopment, wb: [f64; 4]) -> Self {
        if source.dng_development.is_some() {
            #[cfg(feature = "dng-sdk")]
            return Self::dng(wb);
            #[cfg(not(feature = "dng-sdk"))]
            return Self {
                contract: DNG_DEVELOPMENT_CONTRACT.into(),
                ..Self::new("DNG SDK unavailable".into(), wb)
            };
        }
        Self::new(crate::raw::version(), wb)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        require(
            !self.backend_version.is_empty()
                && !self.contract.is_empty()
                && !self.inversion_space.is_empty()
                && !self.working_space.is_empty()
                && self.fixed_wb.iter().all(|v| v.is_finite() && *v > 0.),
            "invalid RAW development identity",
        )?;
        if self.contract == DNG_DEVELOPMENT_CONTRACT {
            require(
                self.fixed_wb[1] == self.fixed_wb[3]
                    && self.fixed_wb[..3]
                        .iter()
                        .copied()
                        .fold(f64::INFINITY, f64::min)
                        == 1.
                    && self.fixed_wb.iter().all(|v| *v <= 256.),
                "SDK WB must use matching greens and the normalized supported camera basis",
            )?;
        }
        Ok(())
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lens_correction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dng_development: Option<String>,
    pub dimensions: [u32; 2],
    pub active_area: [u32; 4],
    pub camera_model: String,
    pub camera_to_ap1: [[f64; 3]; 3],
    /// Developed-source precision, separate from project calibration's native base floor.
    pub input_floor: f64,
}

impl SourceDevelopment {
    pub fn camera_basis(&self) -> CameraBasis {
        CameraBasis {
            camera_model: self.camera_model.clone(),
            camera_to_ap1: self.camera_to_ap1,
        }
    }

    pub(crate) fn processing_calibration(
        &self,
        calibration: Calibration,
        correction: Option<&crate::flat_field::FlatField>,
    ) -> Result<Calibration> {
        calibration.for_source(
            self.input_floor,
            correction.map_or(1., |field| field.bounds().1.max_element()),
        )
    }

    pub(crate) fn validate(&self) -> Result<()> {
        self.camera_basis().validate()?;
        require(
            self.lens_correction.as_ref().is_none_or(|identity| {
                identity.len() == 64
                    && identity
                        .bytes()
                        .all(|v| v.is_ascii_hexdigit() && !v.is_ascii_uppercase())
            }),
            "invalid source lens correction identity",
        )?;
        require(
            self.dng_development.as_ref().is_none_or(|identity| {
                identity.len() == 64
                    && identity
                        .bytes()
                        .all(|v| v.is_ascii_hexdigit() && !v.is_ascii_uppercase())
            }) && !(self.dng_development.is_some() && self.lens_correction.is_some()),
            "invalid or ambiguous DNG development identity",
        )?;
        require(
            self.dimensions.iter().all(|v| *v > 0)
                && contains(
                    [0, 0, self.dimensions[0], self.dimensions[1]],
                    self.active_area,
                ),
            "invalid source dimensions/active area",
        )?;
        require(
            self.input_floor.is_finite()
                && (crate::processing::CAMERA_INPUT_FLOOR
                    ..=crate::processing::CAMERA_INPUT_FLOOR * crate::processing::MAX_SOURCE_GAIN)
                    .contains(&self.input_floor)
                && (self.lens_correction.is_some()
                    || self.dng_development.is_some()
                    || self.input_floor == crate::processing::CAMERA_INPUT_FLOOR),
            "source floor must match the supported native or lens-developed domain",
        )
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
    pub reference_only: bool,
    pub photo: PhotoSettings,
    pub repairs: crate::dust_repair::RepairSet,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_adjustments: Option<LocalAdjustments>,
    #[serde(default, skip_serializing_if = "PhotoOrientation::is_identity")]
    pub orientation: PhotoOrientation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<Crop>,
}

impl Source {
    fn validate(&self) -> Result<()> {
        require(!self.path.as_os_str().is_empty(), "empty source path")?;
        self.development.validate()?;
        if let Some(crop) = self.crop {
            crop.region(self.development.active_area.into())?;
        }
        self.photo.validate()?;
        self.repairs.validate(self.development.active_area.into())?;
        if let Some(locals) = &self.local_adjustments {
            locals.validate()?;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<std::sync::Arc<crate::looks::LookAsset>>,
}

impl Default for Output {
    fn default() -> Self {
        Self {
            contract: OUTPUT_CONTRACT.into(),
            tiff: TiffTarget::DisplayP3,
            exr: ExrPrecision::Half,
            look: None,
        }
    }
}
#[path = "project_flat_field.rs"]
mod flat;
pub use flat::FlatFieldSettings;
#[path = "project_local_adjustments.rs"]
mod local;
pub use local::{AdjustmentId, AdjustmentRecord, LocalAdjustments};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceAnalysis {
    pub source: SourceId,
    pub areas: crate::scan_regions::Areas,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    #[serde(deserialize_with = "deserialize_schema")]
    pub schema: u32,
    pub processing_model: u32,
    pub next_source_id: u64,
    pub development: Option<Development>,
    pub camera_basis: Option<CameraBasis>,
    pub sources: Vec<Source>,
    // Source records survive removal from the ordered filmstrip.
    pub order: Vec<SourceId>,
    pub dmin: Option<[f64; 3]>,
    pub dmax: Option<[f64; 3]>,
    pub input_floor: Option<f64>,
    pub roll: RollSettings,
    #[serde(default, skip_serializing_if = "RollOrientation::is_identity")]
    pub orientation: RollOrientation,
    pub output: Output,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flat_field: Option<FlatFieldSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis_area: Option<[u32; 4]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub analysis_regions: Vec<SourceAnalysis>,
    /// Derived file payload. Runtime document owners detach this from accepted state with
    /// `load_with_spatial_metadata`; snapshots save it separately without changing project equality.
    #[serde(
        default,
        deserialize_with = "crate::spatial_metadata::read_cached",
        skip_serializing_if = "crate::spatial_metadata::empty_or_invalid"
    )]
    pub spatial_metadata: Option<crate::spatial_metadata::Metadata>,
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
            development: None,
            camera_basis: None,
            sources: Vec::new(),
            order: Vec::new(),
            dmin: None,
            dmax: None,
            input_floor: None,
            roll: RollSettings::new_grade(),
            orientation: RollOrientation::default(),
            output: Output::default(),
            flat_field: None,
            analysis_area: None,
            analysis_regions: Vec::new(),
            spatial_metadata: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Save {
    New,
    Replace,
}

impl Project {
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
        source.validate()?;
        require(
            self.development.is_some(),
            "sources require development identity",
        )?;
        self.require_camera_basis(&source.development)?;
        let id = SourceId(self.next_source_id);
        let next = self
            .next_source_id
            .checked_add(1)
            .ok_or_else(|| failure("source ID space exhausted"))?;
        if self.camera_basis.is_none() {
            self.camera_basis = Some(source.development.camera_basis());
        }
        source.id = id;
        self.sources.push(source);
        self.order.push(id);
        self.next_source_id = next;
        Ok(id)
    }

    pub fn require_camera_basis(&self, source: &SourceDevelopment) -> Result<()> {
        self.camera_basis
            .as_ref()
            .map_or(Ok(()), |basis| basis.require_source(source))
    }

    pub fn remove_from_filmstrip(&mut self, id: SourceId) -> Result<()> {
        self.source(id)?;
        self.order.retain(|candidate| *candidate != id);
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
        let (Some(dmin), Some(dmax)) = (self.dmin, self.dmax) else {
            return Ok(None);
        };
        let calibration = Calibration {
            dmin: DVec3::from_array(dmin),
            dmax: DVec3::from_array(dmax),
            input_floor: self
                .input_floor
                .ok_or_else(|| failure("missing calibration floor"))?,
        };
        Ok(Some(calibration.validate()?))
    }

    pub fn active_flat_field(&self) -> Option<&FlatFieldSettings> {
        self.flat_field.as_ref().filter(|field| field.enabled)
    }

    /// Uses the active reference's source bound when available. Without a flat
    /// reference this is only the base calibration; rendering uses a source recipe.
    pub fn processing_calibration(&self) -> Result<Option<Calibration>> {
        self.calibration()?
            .map(|calibration| match self.active_flat_field() {
                Some(field) => field
                    .development
                    .processing_calibration(calibration, Some(&field.map)),
                None => Ok(calibration),
            })
            .transpose()
    }

    pub fn validate(&self) -> Result<()> {
        require(self.schema == SCHEMA_VERSION, "unsupported project schema")?;
        require(
            self.processing_model == self.roll.response.processing_model(),
            "unsupported processing model; no automatic migration",
        )?;
        self.roll.channel_gammas()?;
        wb_matrix(self.roll.wb)?;
        exposure_multiplier(self.roll.exposure)?;
        if let Some(development) = &self.development {
            development.validate()?;
        }
        if let Some(basis) = &self.camera_basis {
            basis.validate()?;
        }
        require(
            self.sources.is_empty() && self.dmin.is_none() && self.dmax.is_none()
                || self.camera_basis.is_some(),
            "sources/calibration require native-camera basis identity",
        )?;
        require(
            !self.output.contract.is_empty(),
            "missing output transform identity",
        )?;
        require(
            self.sources.is_empty() || self.development.is_some(),
            "sources require development identity",
        )?;
        let mut ids = HashSet::new();
        for source in &self.sources {
            require(
                source.id.0 > 0 && source.id.0 < self.next_source_id && ids.insert(source.id),
                "duplicate/invalid source ID allocator",
            )?;
            source.validate()?;
            require(
                source.development.dng_development.is_some()
                    == self
                        .development
                        .as_ref()
                        .is_some_and(|dev| dev.contract == DNG_DEVELOPMENT_CONTRACT),
                "source backend differs from the roll development",
            )?;
            self.require_camera_basis(&source.development)?;
        }
        let mut ordered = HashSet::new();
        for id in &self.order {
            require(
                ids.contains(id) && ordered.insert(*id),
                "invalid/duplicate ordered source ID",
            )?;
        }
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
        if let Some(floor) = self.input_floor {
            require(
                floor == crate::processing::CAMERA_INPUT_FLOOR,
                "project base floor must be one native RGB16 code",
            )?;
        }
        for reference in [self.dmin, self.dmax].into_iter().flatten() {
            require(
                self.development.is_some(),
                "references require development identity",
            )?;
            let floor = self
                .input_floor
                .ok_or_else(|| failure("missing calibration floor"))?;
            require(
                reference.iter().all(|v| v.is_finite() && *v >= floor),
                "invalid stored reference RGB",
            )?;
        }
        if let Some(calibration) = self.calibration()? {
            if self.sources.is_empty() {
                Inversion::new(calibration, self.roll)?;
            }
            for source in &self.sources {
                let calibration = source.development.processing_calibration(
                    calibration,
                    self.active_flat_field().map(|field| field.map.as_ref()),
                )?;
                Inversion::new(calibration, self.roll)?;
            }
        }
        if let Some(field) = &self.flat_field {
            field.validate(self)?;
        }
        Ok(())
    }

    pub fn require_runtime(&self, backend_version: &str) -> Result<()> {
        self.validate()?;
        if let Some(development) = &self.development {
            let matching_backend = development.backend_version == backend_version
                && development.contract == DEVELOPMENT_CONTRACT;
            #[cfg(feature = "dng-sdk")]
            let matching_backend = matching_backend
                || (development.contract == DNG_DEVELOPMENT_CONTRACT
                    && development.backend_version == crate::dng::INTERPRETATION);
            require(
                matching_backend
                    && development.inversion_space == "native-camera-RGB"
                    && development.working_space == "AP1/D60",
                "RAW development contract differs from this runtime",
            )?;
        }
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

    /// Keep disposable region data outside accepted project snapshots/history. Legacy files
    /// have no such payload; unsupported or invalid derived records are unavailable, not grades.
    pub fn load_with_spatial_metadata(
        path: &Path,
    ) -> Result<(Self, crate::spatial_metadata::Metadata)> {
        let mut project = Self::load(path)?;
        let metadata = project
            .spatial_metadata
            .take()
            .unwrap_or_default()
            .for_project(&project);
        Ok((project, metadata))
    }

    /// Capture ready metadata with the write, never install it into the caller's accepted state.
    pub fn save_with_spatial_metadata(
        &self,
        path: &Path,
        policy: Save,
        metadata: &crate::spatial_metadata::Metadata,
    ) -> Result<()> {
        let mut snapshot = self.clone();
        let metadata = metadata.for_project(self);
        snapshot.spatial_metadata = (!metadata.groups.is_empty()).then_some(metadata);
        snapshot.save(path, policy)
    }

    pub fn save(&self, path: &Path, policy: Save) -> Result<()> {
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
        match policy {
            Save::New => install_new(temporary, path)?,
            Save::Replace => {
                temporary.persist(path).map_err(failure)?;
            }
        }
        Ok(())
    }
}

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
#[path = "project_tests.rs"]
mod tests;
