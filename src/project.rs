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
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const SCHEMA_VERSION: u32 = 5;
// Bump when native/raw_bridge.cpp development or scan.rs matrix/normalization/floor contracts change.
pub const DEVELOPMENT_CONTRACT: &str =
    "libraw-camera-ahd-u16-linear-camera-inversion-ap1-bradford-code-floor-v2";
pub const OUTPUT_CONTRACT: &str = "ocio-2.6.0-aces2-100nit-antitone-srgb-displayp3-v2";
const MAX_DOCUMENT_BYTES: u64 = 8 * 1024 * 1024;
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
    fn validate(&self) -> Result<()> {
        require(
            !self.backend_version.is_empty()
                && !self.contract.is_empty()
                && !self.inversion_space.is_empty()
                && !self.working_space.is_empty()
                && self.fixed_wb.iter().all(|v| v.is_finite() && *v > 0.),
            "invalid RAW development identity",
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
    pub dimensions: [u32; 2],
    pub active_area: [u32; 4],
    pub camera_model: String,
    pub camera_to_ap1: [[f64; 3]; 3],
    pub input_floor: f64,
}
impl SourceDevelopment {
    pub fn camera_basis(&self) -> CameraBasis {
        CameraBasis {
            camera_model: self.camera_model.clone(),
            camera_to_ap1: self.camera_to_ap1,
        }
    }
    fn validate(&self) -> Result<()> {
        self.camera_basis().validate()?;
        require(
            self.dimensions.iter().all(|v| *v > 0)
                && contains(
                    [0, 0, self.dimensions[0], self.dimensions[1]],
                    self.active_area,
                ),
            "invalid source dimensions/active area",
        )?;
        require(
            self.input_floor == crate::processing::CAMERA_INPUT_FLOOR,
            "source base floor must be one native RGB16 code",
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
        exposure_multiplier(self.photo.exposure)?;
        wb_matrix(self.photo.wb)?;
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
}
impl Default for Output {
    fn default() -> Self {
        Self {
            contract: OUTPUT_CONTRACT.into(),
            tiff: TiffTarget::DisplayP3,
            exr: ExrPrecision::Half,
        }
    }
}
#[path = "project_flat_field.rs"]
mod flat;
pub use flat::FlatFieldSettings;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
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
    pub fn processing_calibration(&self) -> Result<Option<Calibration>> {
        self.calibration()?
            .map(|calibration| match self.active_flat_field() {
                Some(field) => field.map.processing_calibration(calibration),
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
            self.require_camera_basis(&source.development)?;
        }
        let mut ordered = HashSet::new();
        for id in &self.order {
            require(
                ids.contains(id) && ordered.insert(*id),
                "invalid/duplicate ordered source ID",
            )?;
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
        if let Some(calibration) = self.processing_calibration()? {
            Inversion::new(calibration, self.roll)?;
        }
        if let Some(field) = &self.flat_field {
            field.validate(self)?;
        }
        Ok(())
    }
    pub fn require_runtime(&self, backend_version: &str) -> Result<()> {
        self.validate()?;
        if let Some(development) = &self.development {
            require(
                development.backend_version == backend_version
                    && development.contract == DEVELOPMENT_CONTRACT
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
        let mut bytes = Vec::new();
        File::open(path)
            .map_err(failure)?
            .take(MAX_DOCUMENT_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(failure)?;
        require(
            bytes.len() as u64 <= MAX_DOCUMENT_BYTES,
            "project exceeds the 8 MiB document budget",
        )?;
        let project: Self = serde_json::from_slice(&bytes).map_err(failure)?;
        project.validate()?;
        Ok(project)
    }
    pub fn save(&self, path: &Path, policy: Save) -> Result<()> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self).map_err(failure)?;
        require(
            (bytes.len() as u64) < MAX_DOCUMENT_BYTES,
            "project exceeds the 8 MiB document budget",
        )?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(failure)?;
        temporary.write_all(&bytes).map_err(failure)?;
        temporary.write_all(b"\n").map_err(failure)?;
        temporary.as_file().sync_all().map_err(failure)?;
        match policy {
            Save::New => {
                temporary.persist_noclobber(path).map_err(failure)?;
            }
            Save::Replace => {
                temporary.persist(path).map_err(failure)?;
            }
        }
        Ok(())
    }
}
