use crate::{
    Error,
    project::{Development, Project, Save, Source, SourceDevelopment, SourceId},
    raw::{Metadata, RawFile},
    scan::Scan,
    source_store::{SourceKey, SourceStore},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

type Result<T> = std::result::Result<T, Error>;

fn failure(error: impl std::fmt::Display) -> Error {
    Error::Project(error.to_string())
}

struct CacheScope(Arc<SourceStore>);

impl Drop for CacheScope {
    fn drop(&mut self) {
        self.0.close();
    }
}

#[derive(Clone)]
pub(super) struct Document {
    pub project: Project,
    pub path: Option<PathBuf>,
    pub spatial_metadata: Arc<crate::spatial_metadata::Metadata>,
    saved: Project,
    cache: Arc<CacheScope>,
    parent_cache: Option<Arc<SourceStore>>,
    #[cfg(test)]
    pub(super) fixture_directory: Option<Arc<tempfile::TempDir>>,
}

impl Default for Document {
    fn default() -> Self {
        let project = Project::default();
        Self {
            saved: project.clone(),
            project,
            path: None,
            spatial_metadata: Arc::default(),
            cache: Arc::new(CacheScope(Arc::new(SourceStore::new(0)))),
            parent_cache: None,
            #[cfg(test)]
            fixture_directory: None,
        }
    }
}

impl Document {
    pub fn cache(&self) -> &Arc<SourceStore> {
        &self.cache.0
    }

    pub fn parent_cache(&self) -> Option<&Arc<SourceStore>> {
        self.parent_cache.as_ref()
    }

    pub fn fork_cache(&mut self) {
        self.parent_cache = Some(self.cache.0.clone());
        self.cache = Arc::new(CacheScope(Arc::new(SourceStore::new(0))));
    }

    pub fn accept_cache(&mut self, previous: &Arc<SourceStore>) {
        if let Some(parent) = self.parent_cache.take()
            && Arc::ptr_eq(&parent, previous)
        {
            let allowed = self
                .project
                .development
                .as_ref()
                .map(|development| {
                    self.project
                        .sources
                        .iter()
                        .filter_map(|source| {
                            SourceKey::capture(
                                &self.source_path(source.id).ok()?,
                                &source.development,
                                development,
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();

            parent.transfer_to(&self.cache.0, &allowed);
        }
    }

    pub fn dirty(&self) -> bool {
        self.project != self.saved
    }

    pub fn spatial_code_scale(&self) -> f64 {
        self.project
            .sources
            .iter()
            .filter(|source| !source.reference_only)
            .fold(1f64, |scale, source| {
                scale.max(source.development.input_floor / crate::processing::CAMERA_INPUT_FLOOR)
            })
    }

    /// Cheap freshness queries belong to preparation/publication, not drawing. Measurement omits
    /// the roll flat-field; embedded camera correction remains part of source interpretation.
    pub fn spatial_inputs(&self) -> Vec<crate::spatial_metadata::Input> {
        let code_scale = self.spatial_code_scale();
        self.project
            .sources
            .iter()
            .filter(|source| !source.reference_only)
            .filter_map(|source| {
                Some(crate::spatial_metadata::Input {
                    source: source.id,
                    revision: crate::source_revision::FileRevision::read(
                        &self.source_path(source.id).ok()?,
                    )
                    .ok()?,
                    development: source.development.clone(),
                    correction: None,
                    code_scale,
                })
            })
            .collect()
    }

    pub fn ready_spatial_metadata(&self) -> crate::spatial_metadata::Metadata {
        self.spatial_metadata
            .matching(&self.project, &self.spatial_inputs())
    }

    /// Accept the written snapshot as a baseline, without replacing edits made since dispatch.
    pub fn accept_save(&mut self, saved: Self) {
        rebase_project(
            &mut self.project,
            self.path.as_deref(),
            saved.path.as_deref(),
        );

        self.path = saved.path;
        self.saved = saved.saved;
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

    #[cfg(test)]
    pub fn validate(&self) -> Result<()> {
        self.validate_with_device(None)
    }

    /// Preflight the complete document without publishing grades or retaining offscreen images.
    pub fn validate_for_device(&self, device: &wgpu::Device) -> Result<()> {
        self.validate_with_device(Some(device))
    }

    pub(super) fn validate_with_device(&self, device: Option<&wgpu::Device>) -> Result<()> {
        self.project.require_runtime(&crate::raw::version())?;
        if let Some(device) = device
            && let Some(asset) = &self.project.output.look
        {
            crate::gpu::look::Lut::require_limits(&device.limits(), asset.lut())?;
        }
        let calibration = self.project.calibration()?;
        for source in &self.project.sources {
            if let Some(calibration) = calibration {
                super::processing::validate_source(
                    device,
                    source.development.processing_calibration(
                        calibration,
                        self.project
                            .active_flat_field()
                            .map(|field| field.map.as_ref()),
                    )?,
                    self.project.roll,
                    source,
                    source.photo,
                )?;
            } else if source
                .local_adjustments
                .as_ref()
                .is_some_and(|locals| !locals.adjustments.is_empty())
            {
                return Err(failure(
                    "local adjustments require a calibrated processing basis",
                ));
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
        Self::open_with_progress(path, None, |_, _| {})
    }

    pub fn open_with_progress(
        path: &Path,
        device: Option<&wgpu::Device>,
        mut progress: impl FnMut(usize, usize),
    ) -> Result<Self> {
        let path = absolute(path)?;
        let (project, spatial_metadata) = Project::load_with_spatial_metadata(&path)?;
        let mut document = Self {
            saved: project.clone(),
            project,
            path: Some(path),
            spatial_metadata: Arc::new(spatial_metadata),
            cache: Arc::new(CacheScope(Arc::new(SourceStore::new(0)))),
            parent_cache: None,
            #[cfg(test)]
            fixture_directory: None,
        };
        document.validate_with_device(device)?;
        let total = document.project.order.len();
        progress(0, total);
        for (index, id) in document.project.order.iter().enumerate() {
            let path = document.source_path(*id)?;
            let (actual, _) = read_source(
                &path,
                document.project.development.as_ref().map(|d| d.fixed_wb),
            )?;
            verify_source(document.project.source(*id)?, &actual)?;
            progress(index + 1, total);
        }
        document.spatial_metadata = Arc::new(document.ready_spatial_metadata());
        Ok(document)
    }

    #[cfg(test)]
    pub fn import(&mut self, paths: &[PathBuf]) -> Result<()> {
        self.import_with_progress(paths, None, |_, _| {})
    }

    pub fn import_with_progress(
        &mut self,
        paths: &[PathBuf],
        device: Option<&wgpu::Device>,
        mut progress: impl FnMut(usize, usize),
    ) -> Result<()> {
        let started = std::time::Instant::now();
        progress(0, paths.len());
        for (index, path) in paths.iter().enumerate() {
            let (source, as_shot_wb) =
                read_source(path, self.project.development.as_ref().map(|d| d.fixed_wb))?;
            if let Some(id) = self.find_source(&source.path) {
                verify_source(self.project.source(id)?, &source)?;
                if !self.project.order.contains(&id) {
                    self.project.order.push(id);
                }
                self.project.source_mut(id)?.reference_only = false;
            } else {
                let development = Development::for_source(&source.development, as_shot_wb);
                self.register(source, development)?;
            }
            progress(index + 1, paths.len());
        }

        self.validate_with_device(device)?;
        println!(
            "raw_import sources={} seconds={:.6}",
            paths.len(),
            started.elapsed().as_secs_f64()
        );
        Ok(())
    }

    pub fn register(&mut self, source: Source, development: Development) -> Result<SourceId> {
        if let Some(current) = &self.project.development {
            if current.backend_version != development.backend_version
                || current.contract != development.contract
            {
                return Err(failure(
                    "RAW backend differs; use a separate roll/calibration",
                ));
            }
        } else {
            self.project.development = Some(development);
        }
        if self.project.input_floor.is_none() {
            self.project.input_floor = Some(crate::processing::CAMERA_INPUT_FLOOR);
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

    #[cfg(test)]
    pub fn save(&self, path: &Path, policy: Save) -> Result<Self> {
        self.save_with_device(path, policy, None)
    }

    pub fn save_with_device(
        &self,
        path: &Path,
        policy: Save,
        device: Option<&wgpu::Device>,
    ) -> Result<Self> {
        let path = absolute(path)?;
        // Keep Save-in-place available for existing JSON-named projects.
        if !path.extension().is_some_and(|extension| {
            extension.eq_ignore_ascii_case("antitone") || extension.eq_ignore_ascii_case("json")
        }) {
            return Err(failure(
                "project destination must use an .antitone extension",
            ));
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
        candidate.validate_with_device(device)?;
        let metadata = self.ready_spatial_metadata();
        candidate
            .project
            .save_with_spatial_metadata(&path, policy, &metadata)?;
        candidate.spatial_metadata = Arc::new(metadata);
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

pub(super) fn rebase_project(project: &mut Project, old: Option<&Path>, new: Option<&Path>) {
    if old == new {
        return;
    }
    let old_parent = old.and_then(Path::parent).unwrap_or(Path::new("."));
    let new_parent = new.and_then(Path::parent).unwrap_or(Path::new("."));
    for source in &mut project.sources {
        if source.path.is_relative() {
            // Rebase directories, not RAWs: unavailable non-filmstrip records remain valid.
            source.path = rebase_relative(&source.path, old_parent, new_parent)
                .unwrap_or_else(|_| old_parent.join(&source.path));
        }
    }
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

pub(super) fn metadata_development(metadata: &Metadata) -> Result<SourceDevelopment> {
    let matrix = metadata.camera_to_ap1()?;
    let area = metadata.image_area()?;
    Ok(SourceDevelopment {
        lens_correction: metadata.lens_identity()?,
        dng_development: metadata.dng_identity(),
        dimensions: metadata.source_dimensions()?,
        active_area: [area.x, area.y, area.width, area.height],
        camera_model: metadata.camera_model(),
        camera_to_ap1: matrix.to_cols_array_2d(),
        input_floor: metadata.input_floor()?,
    })
}

pub(super) fn read_source(path: &Path, wb: Option<[f64; 4]>) -> Result<(Source, [f64; 4])> {
    let (source, raw) = open_source(path, wb)?;
    Ok((
        source,
        raw.fixed_wb(wb.unwrap_or_else(|| raw.metadata.as_shot_wb())),
    ))
}

pub(super) fn open_source(path: &Path, wb: Option<[f64; 4]>) -> Result<(Source, RawFile)> {
    let path = absolute(path)?;
    let started = std::time::Instant::now();
    let raw = RawFile::open_with_wb(&path, wb)
        .map_err(|error| failure(format!("{}: {error}", path.display())))?;
    println!(
        "source_registration header_seconds={:.6}",
        started.elapsed().as_secs_f64()
    );
    Ok((source_metadata(&path, &raw.metadata)?, raw))
}

pub(super) fn source_metadata(path: &Path, metadata: &Metadata) -> Result<Source> {
    Ok(Source {
        id: SourceId(0),
        path: absolute(path)?,
        development: metadata_development(metadata)?,
        reference_only: false,
        photo: Default::default(),
        local_adjustments: None,
        repairs: Default::default(),
        orientation: metadata.photo_orientation()?,
        crop: None,
    })
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
        lens_correction: scan.source().metadata().lens_identity()?,
        dng_development: scan.source().metadata().dng_identity(),
        dimensions: [width, height],
        active_area: [area.x, area.y, area.width, area.height],
        camera_model: scan.camera_model.clone(),
        camera_to_ap1: scan.camera_to_ap1.to_cols_array_2d(),
        input_floor: scan.source().input_floor(),
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
