//! Document snapshots, candidate preparation and transactional editor installation.
use super::{ActivityOptions, Editor, history};
use crate::{
    Error,
    project::{Project, Save, Source, SourceId},
};

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub(super) use crate::source::{open_source, source_metadata, verify_scan, verify_source};
type Result<T, E = Error> = std::result::Result<T, E>;

fn failure(error: impl std::fmt::Display) -> Error {
    Error::Project(error.to_string())
}

#[derive(Clone)]
pub(super) struct Document {
    pub project: super::editing::ProjectVersion,
    pub path: Option<PathBuf>,
    pub spatial_metadata: Arc<crate::spatial_metadata::Metadata>,
    pub exposure_metadata: Arc<crate::exposure_metadata::Metadata>,
    pub development_metadata: Arc<crate::development_metadata::Metadata>,
    saved: super::editing::ProjectVersion,

    // Keep identities for undo; each record locator follows project-path rebasing on Save As.
    source_files: std::collections::HashMap<SourceId, (PathBuf, crate::source::FileKey)>,
    #[cfg(test)]
    pub(super) fixture_directory: Option<Arc<tempfile::TempDir>>,
}

impl Default for Document {
    fn default() -> Self {
        let project = super::editing::ProjectVersion::from(Project::default());
        Self {
            saved: project.clone(),
            project,
            path: None,
            spatial_metadata: Arc::default(),
            exposure_metadata: Arc::default(),
            development_metadata: Arc::default(),
            source_files: Default::default(),
            #[cfg(test)]
            fixture_directory: None,
        }
    }
}

#[cfg(test)]
impl Document {
    /// Test fixtures predate per-source acquisition; the decode rides on the source record.
    pub fn register_with_development(
        &mut self,
        mut source: Source,
        development: crate::project::Development,
    ) -> Result<SourceId> {
        source.development.decode = development;
        self.register(source)
    }
}

impl Document {
    pub fn file_key(&self, path: &Path) -> crate::source::FileKey {
        self.project
            .sources
            .iter()
            .find(|source| {
                self.source_path(source.id)
                    .is_ok_and(|resolved| resolved == path)
            })
            .and_then(|source| {
                self.source_files
                    .get(&source.id)
                    .filter(|(locator, _)| *locator == source.path)
            })
            .map(|(_, file)| file.clone())
            .unwrap_or_else(|| crate::source::FileKey::unversioned(path))
    }

    pub fn source_key(&self, id: SourceId) -> Result<crate::source::SourceKey> {
        self.source_key_with(id, &Default::default())
    }

    pub fn source_key_with(
        &self,
        id: SourceId,
        initialization: &crate::source::Initialization,
    ) -> Result<crate::source::SourceKey> {
        self.source_input(id, initialization)?
            .cached_key()
            .ok_or(Error::UnusableSample(
                "source interpretation has not been acquired",
            ))
    }

    pub(super) fn capture_source_files(&mut self) -> Result<()> {
        self.source_files = self
            .project
            .sources
            .iter()
            .map(|source| {
                let path = self.source_path(source.id)?;
                Ok((
                    source.id,
                    (source.path.clone(), crate::source::FileKey::new(&path)),
                ))
            })
            .collect::<Result<_>>()?;
        Ok(())
    }

    fn rebase_source_files(&mut self, old: Option<&Path>, new: Option<&Path>) {
        if old == new {
            return;
        }
        let old_parent = old.and_then(Path::parent).unwrap_or(Path::new("."));
        let new_parent = new.and_then(Path::parent).unwrap_or(Path::new("."));
        for (locator, _) in self.source_files.values_mut() {
            if locator.is_relative() {
                *locator = rebase_relative(locator, old_parent, new_parent)
                    .unwrap_or_else(|_| old_parent.join(&*locator));
            }
        }
    }

    fn inspect_source(
        path: &Path,
        wb: Option<[f64; 4]>,
    ) -> Result<(Source, crate::raw::SourcePreview)> {
        let (source, raw) = open_source(path, wb)?;
        let preview = raw
            .metadata
            .source_preview(source.development.decode.white_balance);
        Ok((source, preview))
    }

    pub(super) fn image_recipe(&self, id: SourceId) -> Result<crate::image_recipe::ImageRecipe> {
        let source = self.project.source(id)?;
        let origin = self.exposure_origin(id).expect("registered photo locator");
        self.project.capture_image(source, |recipe| {
            recipe.neutral.source_development = crate::development_metadata::Development::captured(
                source,
                &origin,
                self.project.input,
                &self.development_metadata,
            );
            recipe.neutral.reference = self.reference_exposure(source).cloned();
        })
    }

    pub(super) fn source_input(
        &self,
        id: SourceId,
        initialization: &crate::source::Initialization,
    ) -> Result<crate::source::Input> {
        let path = self.source_path(id)?;
        let file = self.file_key(&path);
        initialization.input(
            &self.project,
            id,
            path,
            file,
            &self.development_metadata,
            self.project.interpretation_basis(),
        )
    }

    pub(super) fn exposure_origin(&self, id: SourceId) -> Option<PathBuf> {
        self.source_path(id)
            .ok()
            .map(|path| self.file_key(&path).path)
    }

    pub(super) fn exposure_slot(
        &self,
        initialization: &crate::source::Initialization,
        id: SourceId,
    ) -> Arc<std::sync::OnceLock<crate::exposure_reference::ReferenceExposure>> {
        initialization.exposures.memo(
            id,
            &self.exposure_origin(id).expect("registered photo locator"),
        )
    }

    pub(super) fn capture_initialization(
        &mut self,
        initialization: &crate::source::Initialization,
    ) {
        let mut metadata = self.exposure_metadata.as_ref().clone();
        for entry in initialization
            .exposures
            .snapshot(&self.project, |id| self.exposure_origin(id))
            .entries
        {
            metadata
                .entries
                .retain(|existing| existing.source != entry.source);
            metadata.entries.push(entry);
        }
        self.exposure_metadata = Arc::new(metadata.for_project(&self.project));
        let mut developments = initialization
            .developments
            .snapshot(&self.project, |id| self.exposure_origin(id));
        developments.scan_basis = if self.project.scan.is_none() {
            self.scan_basis()
        } else {
            None
        };
        self.development_metadata = Arc::new(developments);
    }

    pub(super) fn reference_exposure(
        &self,
        source: &Source,
    ) -> Option<&crate::exposure_reference::ReferenceExposure> {
        self.exposure_metadata
            .entries
            .iter()
            .find(|entry| entry.source == source.id)
            .map(|entry| &entry.reference)
    }

    fn has_normalization(&self, source: &Source) -> Result<bool> {
        Ok(self.reference_exposure(source).is_some())
    }

    pub(super) fn positive_matrix(
        &self,
        source: &Source,
        input: crate::processing::InputSettings,
    ) -> Result<glam::DMat3> {
        input.positive_matrix(
            glam::DMat3::from_cols_array_2d(
                &self
                    .image_recipe(source.id)?
                    .neutral
                    .source_development
                    .resolved()?
                    .camera_to_ap1,
            ),
            self.reference_exposure(source)
                .map(|reference| reference.ev()),
        )
    }

    pub fn dirty(&self) -> bool {
        self.project != self.saved
    }

    /// Measurement omits the roll flat-field; embedded camera correction remains source interpretation.
    pub fn scan_basis(&self) -> Option<crate::project::ScanBasis> {
        self.project
            .scan_basis()
            .cloned()
            .or_else(|| self.project.interpretation_basis().get().cloned())
    }

    pub fn require_scan_source(
        &self,
        development: &crate::project::SourceDevelopment,
    ) -> Result<()> {
        if self.project.input.requires_scan_sources() {
            if let Some(first) = self
                .project
                .order
                .first()
                .and_then(|id| self.project.source(*id).ok())
                && first.development.camera_model != development.camera_model
            {
                return Err(Error::Project(
                    "sources do not share one scanning setup; use Digital for mixed sources".into(),
                ));
            }
            self.scan_basis()
                .unwrap_or_else(|| crate::project::ScanBasis::from_development(development))
                .require_source(development)?;
        }
        Ok(())
    }

    pub fn spatial_inputs(&self) -> Vec<crate::spatial_metadata::Input> {
        self.spatial_inputs_with(&Default::default())
    }

    pub fn spatial_inputs_with(
        &self,
        initialization: &crate::source::Initialization,
    ) -> Vec<crate::spatial_metadata::Input> {
        self.project
            .sources
            .iter()
            .filter_map(|source| {
                Some(crate::spatial_metadata::Input {
                    source: source.id,
                    revision: self.file_key(&self.source_path(source.id).ok()?).revision?,
                    development: self
                        .source_input(source.id, initialization)
                        .ok()?
                        .record()
                        .ok()?
                        .development,
                    correction: None,
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
        let old_path = self.path.clone();
        rebase_project(
            &mut self.project,
            self.path.as_deref(),
            saved.path.as_deref(),
        );

        self.path = saved.path;
        self.saved = saved.saved;
        let new_path = self.path.clone();
        self.rebase_source_files(old_path.as_deref(), new_path.as_deref());
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

    /// Preflight known source domains without retaining offscreen images; unresolved
    /// Digital domains complete numerical admission during lazy preparation.
    pub fn validate_for_device(&self, device: &wgpu::Device) -> Result<()> {
        self.validate_with_device(Some(device))
    }

    fn validate_structure_with_device(&self, device: Option<&wgpu::Device>) -> Result<()> {
        self.project.require_runtime()?;
        if let Some(device) = device {
            for asset in self.project.output.luts().assets() {
                crate::gpu::lut3d::Lut::require_limits(&device.limits(), asset.lut())?;
            }
        }
        Ok(())
    }

    pub(super) fn validate_with_device(&self, device: Option<&wgpu::Device>) -> Result<()> {
        self.validate_structure_with_device(device)?;
        let input = self.project.input;
        for source in &self.project.sources {
            // Unvisited Digital sources have no resolved AP1 domain yet. Their layer
            // controls are structurally valid; numerical admission follows preparation.
            if self
                .image_recipe(source.id)?
                .neutral
                .source_development
                .resolved()
                .is_err()
                || (!input.is_negative() && !self.has_normalization(source)?)
            {
                continue;
            }
            {
                let refined = input;
                super::processing::validate_source_with_matrix(
                    device,
                    refined,
                    self.project.roll,
                    source,
                    self.positive_matrix(source, refined)?,
                )?;
            }
        }
        Ok(())
    }

    pub fn open(path: &Path) -> Result<Self> {
        let path = absolute(path)?;
        let mut project = Project::load(&path)?;
        let exposure_metadata = project
            .exposure_metadata
            .take()
            .unwrap_or_default()
            .for_project(&project);
        let mut project = super::editing::ProjectVersion::from(project);
        let development_metadata = project.development_metadata.take().unwrap_or_default();
        let mut document = Self {
            saved: project.clone(),
            project,
            path: Some(path),
            spatial_metadata: Arc::default(),
            exposure_metadata: Arc::new(exposure_metadata),
            development_metadata: Arc::new(development_metadata),
            source_files: Default::default(),
            #[cfg(test)]
            fixture_directory: None,
        };
        document.validate_with_device(None)?;
        // Best-effort cache freshness only: missing files do not reject metadata.
        document.capture_source_files()?;
        Ok(document)
    }

    #[cfg(test)]
    pub fn import(&mut self, paths: &[PathBuf]) -> Result<()> {
        self.import_with_progress(
            paths,
            &crate::cancellation::Cancellation::default(),
            |_, _| {},
        )
        .map(|_| ())
    }

    pub fn import_with_progress(
        &mut self,
        paths: &[PathBuf],
        cancelled: &crate::cancellation::Cancellation,
        mut progress: impl FnMut(usize, usize),
    ) -> Result<super::viewer_ui::SourcePreviews> {
        let mut previews = super::viewer_ui::SourcePreviews::default();
        progress(0, paths.len());
        let auto_digital = self.auto_digital_available();
        for (index, path) in paths.iter().enumerate() {
            if cancelled.is_cancelled() {
                return Err(Error::Cancelled);
            }
            if self.find_source(path).is_none() {
                let wb = self.project.acquisition_wb();
                let (mut source, mut preview) = Self::inspect_source(path, wb)?;
                if auto_digital
                    && self.project.input.requires_scan_sources()
                    && self.scan_source_incompatible(&source)
                {
                    let mut project = self.project.as_ref().clone();
                    project.input = crate::processing::InputSettings::Digital;
                    project.scan = None;
                    self.project = self.project.updated(Arc::new(project));
                    (source, preview) = Self::inspect_source(path, None)?;
                }
                let id = self.register(source)?;
                previews.insert(self.source_key(id)?, preview);
            }
            progress(index + 1, paths.len());
        }
        if cancelled.is_cancelled() {
            return Err(Error::Cancelled);
        }
        self.project.require_runtime()?;
        Ok(previews)
    }

    fn auto_digital_available(&self) -> bool {
        self.project.input.requires_scan_sources()
            && self.project.sources.len() <= 1
            && !self.project.input.has_stage_settings()
            && self.project.flat_field().is_none()
    }

    fn scan_source_incompatible(&self, source: &Source) -> bool {
        self.scan_basis()
            .is_some_and(|basis| basis.require_source(&source.development).is_err())
    }

    pub fn register(&mut self, source: Source) -> Result<SourceId> {
        if self.project.input.requires_scan_sources()
            && let Some(basis) = self.scan_basis()
        {
            basis.require_source(&source.development)?;
        }
        let id = self.project.add_source(source)?;
        let path = self.source_path(id)?;
        self.source_files.insert(
            id,
            (
                self.project.source(id)?.path.clone(),
                crate::source::FileKey::new(&path),
            ),
        );
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
        self.validate_with_device(device)?;
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

        let metadata = self.ready_spatial_metadata();
        let mut snapshot = candidate.project.clone();
        let exposure = self.exposure_metadata.for_project(&self.project);
        snapshot.exposure_metadata = (!exposure.entries.is_empty()).then_some(exposure);
        let development = self.development_metadata.for_project(&self.project);
        snapshot.development_metadata = (!development.entries.is_empty()
            || development.scan_basis.is_some())
        .then_some(development);
        snapshot.save(&path, policy)?;
        candidate.spatial_metadata = Arc::new(metadata);
        candidate.rebase_source_files(self.path.as_deref(), Some(&path));
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
            // Resolve directories without requiring the photo itself to exist.
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

#[cfg(test)]
pub(super) fn read_source(path: &Path, wb: Option<[f64; 4]>) -> Result<Source> {
    open_source(path, wb).map(|(source, _)| source)
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod workflow_tests;

pub(super) struct Restore {
    pub document: Document,
    pub selected: Option<SourceId>,
    pub direction: history::Direction,
}

pub(super) struct Prepared {
    pub document: Document,
    pub selected: Option<SourceId>,
}

pub(super) struct Write {
    pub document: Document,
    pub path: PathBuf,
    pub policy: Save,
}

impl Write {
    pub fn save(self, device: &wgpu::Device) -> Result<Document, Error> {
        self.document
            .save_with_device(&self.path, self.policy, Some(device))
    }
}

fn prepare_restore(
    mut restore: Restore,
    initialization: Arc<crate::source::Initialization>,
) -> Result<Prepared, Error> {
    restore.document.capture_initialization(&initialization);
    restore.document.validate_with_device(None)?;
    Ok(Prepared {
        document: restore.document,
        selected: restore.selected,
    })
}

#[derive(Default)]
pub(super) struct RestoreTask {
    task: Option<async_executor::Task<()>>,
}

impl Editor {
    pub(super) fn start_history(
        &mut self,
        restore: Restore,
        cx: &crate::application::UiContext<'_, Self>,
    ) {
        if self.dust_search_pending() {
            self.status = "Wait for donor selection before changing the document".into();
            return;
        }
        self.cancel_spatial();
        self.cancel_control_edit();
        self.history_pending = Some(history::Pending {
            direction: restore.direction,
            cleanup: self.dusting().then(|| self.dust_outlines()),
        });
        self.cancel_active_tool();
        self.document.visit.masks.cancel();
        let id = self.begin_document_operation();
        let activity = self
            .activities
            .start(ActivityOptions::new("Restoring edit…").modal());
        let initialization = self.document.initialization.clone();
        self.history_restore.task = cx.spawn(move |cx| async move {
            let result = crate::background::run(move |_| prepare_restore(restore, initialization))
                .await
                .map_err(|_| "History restoration worker panicked".to_owned())
                .and_then(|result| result.map(Box::new).map_err(|error| error.to_string()));
            drop(activity);
            cx.update(|editor| {
                editor.history_restored(id, result, &cx);
                editor.acquire_working(&cx);
            });
        });
    }

    pub(super) fn history_restored(
        &mut self,
        id: u64,
        result: Result<Box<Prepared>, String>,
        cx: &crate::application::UiContext<'_, Self>,
    ) {
        if self.document_pending != Some(id) {
            return;
        }
        self.history_restore = RestoreTask::default();
        let pending_history = self.history_pending.take();
        self.document_pending = None;
        match result {
            Ok(prepared) => {
                let next = prepared.selected.and_then(|source| {
                    prepared
                        .document
                        .source_input(source, &self.document.initialization)
                        .ok()
                });
                let same_visit = self.document.visit.current_source == prepared.selected
                    && match (&next, &self.document.visit.canvas.ready) {
                        (Some(input), Some(ready)) => {
                            input.matches(&ready.path, &ready.file, &ready.record)
                        }
                        (Some(input), None) => self
                            .document
                            .visit
                            .source
                            .input
                            .as_ref()
                            .is_some_and(|current| current.same_request(input)),
                        (None, _) => true,
                    };
                let correction_changed = self.document.project.active_flat_field()
                    != prepared.document.project.active_flat_field();
                self.document.install(prepared.document);
                self.remember_imported_look();
                self.id = id;
                if !same_visit {
                    if self.document.visit.current_source == prepared.selected {
                        self.reinterpret_photo(id);
                    } else {
                        self.replace_photo_scope(id);
                    }
                }
                self.document.visit.current_source = prepared.selected;
                self.document
                    .roll_selection
                    .reconcile(&self.document.state.project.order, prepared.selected);
                self.document.selected = prepared
                    .selected
                    .and_then(|source| {
                        self.document
                            .project
                            .order
                            .iter()
                            .position(|candidate| *candidate == source)
                    })
                    .unwrap_or(0);
                self.refresh_adjustment_layer_selection();
                self.invalidate_original();
                if correction_changed {
                    self.document.correction_restore = None;
                    self.document.correction = Default::default();
                    self.restore_correction(cx);
                }
                self.history_prepared(pending_history);
                self.observe_gamma_pivot();
                if !same_visit && let Some(input) = next {
                    self.start_source_acquisition(input.path().to_owned(), cx);
                } else {
                    self.resume_source_after_document();
                }
            }
            Err(error) => {
                self.history_failed(pending_history);
                self.resume_source_after_document();
                self.status = error;
                if self.document.visit.source.pending() {
                    self.document.visit.return_notice = Some(self.status.clone());
                }
                self.source_epoch
                    .store(self.id, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
}
