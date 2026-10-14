use crate::{
    Error,
    gpu::ProcessingParameters,
    project::{Development, Project, Save, Source, SourceDevelopment, SourceId},
    raw::{Metadata, RawFile},
    scan::Scan,
};
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Error>;
fn failure(error: impl std::fmt::Display) -> Error {
    Error::Project(error.to_string())
}

#[derive(Clone)]
pub(super) struct Document {
    pub project: Project,
    pub path: Option<PathBuf>,
    saved: Project,
}
impl Default for Document {
    fn default() -> Self {
        let project = Project::default();
        Self {
            saved: project.clone(),
            project,
            path: None,
        }
    }
}
impl Document {
    pub fn dirty(&self) -> bool {
        self.project != self.saved
    }
    pub fn source_path(&self, id: SourceId) -> Result<PathBuf> {
        self.project
            .resolve_path(id, self.path.as_deref().unwrap_or(Path::new(".")))
    }
    pub fn find_source(&self, path: &Path) -> Option<SourceId> {
        self.project.sources.iter().find_map(|source| {
            self.source_path(source.id)
                .ok()
                .filter(|candidate| same_path(candidate, path))
                .map(|_| source.id)
        })
    }
    pub fn validate(&self) -> Result<()> {
        self.project.require_runtime(&crate::raw::version())?;
        if let Some(calibration) = self.project.processing_calibration()? {
            for source in &self.project.sources {
                ProcessingParameters::positive(
                    calibration,
                    self.project.roll,
                    source.photo,
                    glam::DMat3::from_cols_array_2d(&source.development.camera_to_ap1),
                )?;
            }
        }
        if let Some(development) = &self.project.development
            && development
                .fixed_wb
                .iter()
                .any(|value| !(*value as f32).is_finite() || *value as f32 <= 0.)
        {
            return Err(failure(
                "saved RAW white balance cannot be represented by the developer",
            ));
        }
        Ok(())
    }
    #[cfg(test)]
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with_progress(path, |_, _| {})
    }
    pub fn open_with_progress(path: &Path, mut progress: impl FnMut(usize, usize)) -> Result<Self> {
        let path = absolute(path)?;
        let project = Project::load(&path)?;
        let document = Self {
            saved: project.clone(),
            project,
            path: Some(path),
        };
        document.validate()?;
        let total = document.project.order.len();
        progress(0, total);
        for (index, id) in document.project.order.iter().enumerate() {
            let path = document.source_path(*id)?;
            let (actual, _) = read_source(&path)?;
            verify_source(document.project.source(*id)?, &actual)?;
            progress(index + 1, total);
        }
        Ok(document)
    }
    #[cfg(test)]
    pub fn import(&mut self, paths: &[PathBuf]) -> Result<()> {
        self.import_with_progress(paths, |_, _| {})
    }
    pub fn import_with_progress(
        &mut self,
        paths: &[PathBuf],
        mut progress: impl FnMut(usize, usize),
    ) -> Result<()> {
        let started = std::time::Instant::now();
        progress(0, paths.len());
        for (index, path) in paths.iter().enumerate() {
            let (source, wb) = read_source(path)?;
            if let Some(id) = self.find_source(&source.path) {
                verify_source(self.project.source(id)?, &source)?;
                if !self.project.order.contains(&id) {
                    self.project.order.push(id);
                }
                self.project.source_mut(id)?.reference_only = false;
            } else {
                self.register(source, Development::new(crate::raw::version(), wb))?;
            }
            progress(index + 1, paths.len());
        }
        self.validate()?;
        println!(
            "raw_import sources={} seconds={:.6}",
            paths.len(),
            started.elapsed().as_secs_f64()
        );
        Ok(())
    }
    pub fn register(&mut self, source: Source, development: Development) -> Result<SourceId> {
        if self.project.development.is_none() {
            self.project.development = Some(development);
        }
        if self.project.input_floor.is_none() {
            self.project.input_floor = Some(source.development.input_floor);
        }
        self.project.add_source(source)
    }
    pub fn register_reference(
        &mut self,
        mut source: Source,
        development: Development,
    ) -> Result<SourceId> {
        if let Some(id) = self.find_source(&source.path) {
            verify_source(self.project.source(id)?, &source)?;
            return Ok(id);
        }
        source.reference_only = true;
        let id = self.register(source, development)?;
        self.project.remove_from_filmstrip(id)?;
        Ok(id)
    }
    pub fn save(&self, path: &Path, policy: Save) -> Result<Self> {
        let path = absolute(path)?;
        if !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        {
            return Err(failure("project destination must use a .json extension"));
        }
        let mut candidate = self.clone();
        for source in &mut candidate.project.sources {
            let resolved = self.source_path(source.id)?;
            if same_path(&path, &resolved) {
                return Err(failure("project destination is an input RAW"));
            }
            if self.path.as_ref() != Some(&path) && source.path.is_relative() {
                let old_parent = self
                    .path
                    .as_ref()
                    .and_then(|path| path.parent())
                    .unwrap_or(Path::new("."));
                source.path = rebase_relative(
                    &source.path,
                    old_parent,
                    path.parent().expect("absolute destination parent"),
                )?;
            }
        }
        candidate.validate()?;
        candidate.project.save(&path, policy)?;
        candidate.path = Some(path);
        candidate.saved = candidate.project.clone();
        Ok(candidate)
    }
}

pub(super) fn absolute(path: &Path) -> Result<PathBuf> {
    Ok(if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_err(failure)?.join(path)
    })
}
pub(super) fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    if let (Ok(a), Ok(b)) = (absolute(a), absolute(b))
        && a == b
    {
        return true;
    }
    // Missing source records are valid; canonical paths only supplement locator equality.
    matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
}
fn rebase_relative(path: &Path, old_parent: &Path, new_parent: &Path) -> Result<PathBuf> {
    let directory = |path: &Path| match path.canonicalize() {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => absolute(path),
        Err(error) => Err(failure(error)),
    };
    let target = directory(old_parent)?.join(path);
    let base = directory(new_parent)?;
    let target_parts: Vec<_> = target.components().collect();
    let base_parts: Vec<_> = base.components().collect();
    if target_parts.first() != base_parts.first() {
        return Ok(target);
    }
    let shared = target_parts
        .iter()
        .zip(&base_parts)
        .take_while(|(a, b)| a == b)
        .count();
    let mut relative = PathBuf::new();
    for _ in &base_parts[shared..] {
        relative.push("..");
    }
    for part in &target_parts[shared..] {
        relative.push(part.as_os_str());
    }
    Ok(relative)
}
fn metadata_development(metadata: &Metadata) -> Result<SourceDevelopment> {
    let matrix = metadata.camera_to_ap1()?;
    let area = metadata.image_area()?;
    Ok(SourceDevelopment {
        dimensions: [metadata.width, metadata.height],
        active_area: [area.x, area.y, area.width, area.height],
        camera_model: metadata.camera_model(),
        camera_to_ap1: matrix.to_cols_array_2d(),
        input_floor: crate::processing::CAMERA_INPUT_FLOOR,
    })
}
pub(super) fn read_source(path: &Path) -> Result<(Source, [f64; 4])> {
    let (source, raw) = open_source(path)?;
    Ok((source, raw.metadata.as_shot_wb()))
}
pub(super) fn open_source(path: &Path) -> Result<(Source, RawFile)> {
    let path = absolute(path)?;
    let started = std::time::Instant::now();
    let raw =
        RawFile::open(&path).map_err(|error| failure(format!("{}: {error}", path.display())))?;
    println!(
        "source_registration header_seconds={:.6}",
        started.elapsed().as_secs_f64()
    );
    Ok((
        Source {
            id: SourceId(0),
            path,
            development: metadata_development(&raw.metadata)?,
            reference_only: false,
            photo: Default::default(),
            orientation: Default::default(),
            crop: None,
        },
        raw,
    ))
}
pub(super) fn verify_source(expected: &Source, actual: &Source) -> Result<()> {
    if expected.development != actual.development {
        return Err(failure(format!(
            "{}: RAW development identity differs",
            actual.path.display()
        )));
    }
    Ok(())
}
pub(super) fn verify_scan(source: &Source, scan: &Scan) -> Result<()> {
    let (width, height) = scan.dimensions()?;
    let area = scan.area;
    let actual = SourceDevelopment {
        dimensions: [width, height],
        active_area: [area.x, area.y, area.width, area.height],
        camera_model: scan.camera_model.clone(),
        camera_to_ap1: scan.camera_to_ap1.to_cols_array_2d(),
        input_floor: scan.input_floor,
    };
    if source.development != actual {
        return Err(failure(
            "developed RAW differs from its recorded header identity",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "editor_document_tests.rs"]
mod tests;
