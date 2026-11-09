//! Document snapshots, candidate preparation and transactional editor installation.
use super::{
    ActivityOptions, AdjustmentLayerEditor, Correction, Editor, Ready, RollSelection, dust,
    history, source,
};
use crate::{
    Error,
    project::{Project, Save, Source, SourceId},
    raw::Metadata,
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
    saved: super::editing::ProjectVersion,

    source_previews: std::collections::HashMap<SourceId, crate::raw::SourcePreview>,

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
            source_previews: Default::default(),
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

    pub fn register_reference_with_development(
        &mut self,
        mut source: Source,
        development: crate::project::Development,
    ) -> Result<SourceId> {
        source.development.decode = development;
        self.register_reference(source)
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

    pub(super) fn source_wb(&self, id: SourceId) -> Option<[f64; 4]> {
        self.project
            .source(id)
            .ok()
            .map(|source| source.development.decode.white_balance)
    }

    pub fn source_key(&self, id: SourceId) -> Result<crate::source::SourceKey> {
        let source = self.project.source(id)?;
        let path = self.source_path(id)?;
        Ok(crate::source::SourceKey::for_load(
            &path,
            self.file_key(&path),
            &source.development,
        ))
    }

    pub async fn acquire_source(
        &self,
        id: SourceId,
        sources: &dyn crate::source::SourceProvider,
        exposures: &crate::exposure_metadata::Store,
        cancelled: &crate::cancellation::Cancellation,
    ) -> Result<Option<crate::scan::Scan>> {
        if cancelled.is_cancelled() {
            return Ok(None);
        }
        self.project.source(id)?.grade_snapshot()?;
        let key = self.source_key(id)?;
        let reference = self.exposure_slot(exposures, id);
        let load = crate::photo_preparation::acquire(
            sources,
            key,
            crate::source::Priority::Foreground,
            self.project.input,
            reference,
        );
        let result = futures_lite::future::or(
            async {
                cancelled.wait().await;
                Ok(None)
            },
            async { load.await.map(|prepared| Some(prepared.scan())) },
        )
        .await;
        if cancelled.is_cancelled() {
            Ok(None)
        } else {
            result
        }
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

    pub(super) fn remember_source_preview(
        &mut self,
        id: SourceId,
        metadata: &Metadata,
        white_balance: [f64; 4],
    ) {
        self.source_previews
            .insert(id, metadata.source_preview(white_balance));
    }

    pub(super) fn source_preview(&self, source: &Source) -> crate::raw::SourcePreview {
        self.source_previews
            .get(&source.id)
            .copied()
            .unwrap_or_default()
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
        self.project.capture_image(source, |recipe| {
            recipe.neutral.reference = self.reference_exposure(source).cloned();
        })
    }

    pub(super) fn exposure_origin(&self, id: SourceId) -> Option<PathBuf> {
        self.source_path(id)
            .ok()
            .map(|path| self.file_key(&path).path)
    }

    pub(super) fn exposure_slot(
        &self,
        exposures: &crate::exposure_metadata::Store,
        id: SourceId,
    ) -> Arc<std::sync::OnceLock<crate::exposure_reference::ReferenceExposure>> {
        exposures.memo(
            id,
            &self.exposure_origin(id).expect("registered photo locator"),
        )
    }

    pub(super) fn capture_exposure(&mut self, exposures: &crate::exposure_metadata::Store) {
        let mut metadata = self.exposure_metadata.as_ref().clone();
        for entry in exposures
            .snapshot(&self.project, |id| self.exposure_origin(id))
            .entries
        {
            metadata
                .entries
                .retain(|existing| existing.source != entry.source);
            metadata.entries.push(entry);
        }
        self.exposure_metadata = Arc::new(metadata.for_project(&self.project));
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
            glam::DMat3::from_cols_array_2d(&source.development.camera_to_ap1),
            self.reference_exposure(source)
                .map(|reference| reference.ev()),
        )
    }

    pub(super) fn refresh_source(&mut self, id: SourceId) -> Result<()> {
        let expected = self.project.source(id)?;
        let (actual, preview) = Self::inspect_source(
            &self.source_path(id)?,
            Some(expected.development.decode.white_balance),
        )?;
        if expected.development.decode.backend_version != actual.development.decode.backend_version
        {
            log::debug!(recorded = expected.development.decode.version(), current = actual.development.decode.version(); "source_decoder_changed");
        }
        self.project.source_mut(id)?.development = actual.development;
        self.source_previews.insert(id, preview);
        Ok(())
    }

    pub fn dirty(&self) -> bool {
        self.project != self.saved
    }

    /// Measurement omits the roll flat-field; embedded camera correction remains source interpretation.
    pub fn spatial_inputs(&self) -> Vec<crate::spatial_metadata::Input> {
        self.project
            .sources
            .iter()
            .filter(|source| !source.reference_only)
            .filter_map(|source| {
                Some(crate::spatial_metadata::Input {
                    source: source.id,
                    revision: self.file_key(&self.source_path(source.id).ok()?).revision?,
                    development: source.development.clone(),
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
            if !input.is_negative() && !self.has_normalization(source)? {
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
        let mut project = Project::load(&path)?;
        let exposure_metadata = project
            .exposure_metadata
            .take()
            .unwrap_or_default()
            .for_project(&project);
        let project = super::editing::ProjectVersion::from(project);
        let mut document = Self {
            saved: project.clone(),
            project,
            path: Some(path),
            spatial_metadata: Arc::default(),
            exposure_metadata: Arc::new(exposure_metadata),
            source_previews: Default::default(),
            source_files: Default::default(),
            #[cfg(test)]
            fixture_directory: None,
        };
        document.capture_source_files()?;
        let total = document.project.order.len();
        progress(0, total);
        for (index, id) in document.project.order.clone().into_iter().enumerate() {
            document.refresh_source(id)?;
            progress(index + 1, total);
        }
        if document.project.input.is_negative() {
            document.project.reset_scan_basis();
        }

        document.validate_with_device(device)?;
        document.saved = document.project.clone();
        Ok(document)
    }

    /// Re-acquire active sources for the target input stage before accepting a stage switch.
    /// Negative uses unity WB; Digital resolves each source's as-shot WB. Crossing stages clears
    /// the accepted correction and its reference record before this redevelopment.
    pub(super) fn redevelop_for_input(
        &mut self,
        target: crate::processing::InputSettings,
    ) -> Result<()> {
        // A correction belongs to the stage in which it was measured, so crossing stages drops it.
        self.project.set_flat_field(None);
        let wb = target.is_negative().then_some(crate::project::UNITY_WB);
        let active = self.project.active_source_ids();
        let mut developments = Vec::with_capacity(active.len());
        for id in active {
            let path = self.source_path(id)?;
            let (source, preview) = Self::inspect_source(&path, wb)?;
            developments.push((id, source.development, preview));
        }
        for (id, development, preview) in developments {
            self.project.source_mut(id)?.development = development;
            self.source_previews.insert(id, preview);
        }
        self.project.input = target;
        if target.requires_scan_sources() {
            self.project.reset_scan_basis();
            self.project.require_scan_sources()?;
        }
        Ok(())
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
        let auto_digital = self.auto_digital_available();
        // Negative decodes every new source with unity WB; Digital uses each source's as-shot
        // WB. The first successful scan establishes the compatibility basis inside
        // `add_source`, so read it again for every file.
        for (index, path) in paths.iter().enumerate() {
            let existing = self.find_source(path);
            if let Some(id) = existing {
                if self.project.order.contains(&id) {
                    self.refresh_source(id)?;
                } else {
                    // The only nonordered record kept by accepted state is the current
                    // flat-field reference. Importing it as a photo reuses that record.
                    let wb = self
                        .project
                        .input
                        .requires_scan_sources()
                        .then_some(crate::project::UNITY_WB);
                    let (source, preview) = Self::inspect_source(path, wb)?;
                    self.project.order.push(id);
                    self.project.source_mut(id)?.development = source.development;
                    self.source_previews.insert(id, preview);
                }
                self.project.source_mut(id)?.reference_only = false;
                let resolved = self.source_path(id)?;
                self.source_files.insert(
                    id,
                    (
                        self.project.source(id)?.path.clone(),
                        crate::source::FileKey::new(&resolved),
                    ),
                );
            } else {
                let wb = self.project.acquisition_wb();
                let (mut source, mut preview) = Self::inspect_source(path, wb)?;
                // A fresh scan import may contain sources that cannot share one acquisition
                // basis. In that case choose Digital for the batch; the first source remains
                // valid in its own as-shot basis. An existing calibrated Negative project is
                // never switched silently.
                if auto_digital
                    && self.project.input.requires_scan_sources()
                    && self.scan_source_incompatible(&source)
                {
                    self.project.input = crate::processing::InputSettings::Digital;
                    self.project.scan = None;
                    // Every ordered source now belongs to Digital, including one that was
                    // already present before this batch. Reacquire all of them in as-shot WB.
                    self.redevelop_for_input(crate::processing::InputSettings::Digital)?;
                    (source, preview) = Self::inspect_source(path, None)?;
                }
                let id = self.register(source)?;
                self.source_previews.insert(id, preview);
            }
            progress(index + 1, paths.len());
        }

        self.validate_with_device(device)?;
        log::info!(
            sources = paths.len(),
            seconds = started.elapsed().as_secs_f64();
            "raw_import"
        );

        Ok(())
    }

    fn auto_digital_available(&self) -> bool {
        self.project.input.requires_scan_sources()
            && self.project.sources.len() <= 1
            && !self.project.input.has_stage_settings()
            && self.project.flat_field().is_none()
    }

    fn scan_source_incompatible(&self, source: &Source) -> bool {
        self.project
            .require_scan_source(&source.development)
            .is_err()
    }

    pub fn register(&mut self, source: Source) -> Result<SourceId> {
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

    pub fn register_reference(&mut self, mut source: Source) -> Result<SourceId> {
        if let Some(id) = self.find_source(&source.path) {
            verify_source(self.project.source(id)?, &source)?;
            return Ok(id);
        }
        source.reference_only = true;
        let id = self.register(source)?;
        self.project.remove_from_order(id)?;
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

#[cfg(test)]
pub(super) fn read_source(path: &Path, wb: Option<[f64; 4]>) -> Result<Source> {
    open_source(path, wb).map(|(source, _)| source)
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod workflow_tests;

pub(super) enum Preparation {
    Import(Document, Vec<PathBuf>, Option<(u64, SourceId)>),
    Open(PathBuf),
    New,
    Input(Document, Option<SourceId>),
    History(Document, Option<SourceId>, history::Direction),
}

pub(super) struct Request {
    pub work: Preparation,
    pub source: Option<(u64, Ready)>,
    pub correction: Option<Arc<Correction>>,
    pub sources: Option<Arc<dyn crate::source::SourceProvider>>,
    pub exposures: Option<Arc<crate::exposure_metadata::Store>>,
}

pub(super) struct Prepared {
    pub document: Document,
    pub ready: Option<Ready>,
    pub selected: Option<SourceId>,
    pub reuse_ready: bool,
    pub correction: Option<Arc<Correction>>,
    pub preserve_view: bool,
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

pub(super) struct Candidate {
    pub prepared: Prepared,
    pub source: Option<source::Request>,
}

/// The synchronous admission/preparation stage contains metadata, validation and
/// GPU preflight only. Selected native development is a separate awaited service.
pub(super) fn candidate(
    request: Request,
    device: &wgpu::Device,
    mut progress: impl FnMut(usize, usize),
) -> Result<Candidate, Error> {
    let Request {
        work,
        source: previous,
        correction: previous_correction,
        sources,
        exposures,
    } = request;
    let preserve_view = matches!(work, Preparation::Input(..) | Preparation::History(..));
    let selected_override = match &work {
        Preparation::Input(_, selected) | Preparation::History(_, selected, _) => Some(*selected),
        _ => None,
    };
    let (mut document, reuse) = match work {
        Preparation::Import(mut document, paths, reuse) => {
            document.import_with_progress(&paths, Some(device), &mut progress)?;
            (document, reuse)
        }
        Preparation::Open(path) => (
            Document::open_with_progress(&path, Some(device), progress)?,
            None,
        ),
        Preparation::New => (Document::default(), None),
        Preparation::Input(mut document, _) => {
            let target = document.project.input;
            document.redevelop_for_input(target)?;
            (document, None)
        }
        Preparation::History(mut document, _, _) => {
            if let Some(exposures) = &exposures {
                document.capture_exposure(exposures);
            }
            document.validate_for_device(device)?;
            for id in document.project.order.clone() {
                document.refresh_source(id)?;
            }
            (document, None)
        }
    };
    if let Some(exposures) = &exposures {
        document.capture_exposure(exposures);
    }
    let correction = Correction::prepare(device, &document, previous_correction.as_ref())?;
    for record in &document.project.sources {
        crate::gpu::parameters::CameraMatrix::require_source(
            glam::DMat3::from_cols_array_2d(&record.development.camera_to_ap1),
            false,
        )?;
    }
    if let Some((loaded, selected)) = reuse
        && let Some((_, ready)) = previous.filter(|(active, _)| *active == loaded)
        && document
            .project
            .source(selected)
            .is_ok_and(|source| source.development == ready.record.development)
        && (document.project.input.is_negative() || ready.reference.is_some())
    {
        return Ok(Candidate {
            prepared: Prepared {
                document,
                ready: Some(ready),
                selected: Some(selected),
                reuse_ready: true,
                correction,
                preserve_view,
            },
            source: None,
        });
    }
    let selected = selected_override.unwrap_or_else(|| {
        reuse
            .map(|(_, selected)| selected)
            .or_else(|| document.project.order.first().copied())
    });
    let source = selected
        .map(|id| {
            source_request(
                &document,
                id,
                correction.clone(),
                sources.clone(),
                exposures.as_deref(),
            )
        })
        .transpose()?;
    Ok(Candidate {
        prepared: Prepared {
            document,
            ready: None,
            selected,
            reuse_ready: false,
            correction,
            preserve_view,
        },
        source,
    })
}

fn source_request(
    document: &Document,
    id: SourceId,
    correction: Option<Arc<Correction>>,
    sources: Option<Arc<dyn crate::source::SourceProvider>>,
    exposures: Option<&crate::exposure_metadata::Store>,
) -> Result<source::Request, Error> {
    let path = document.source_path(id)?;
    let expected = document.project.source(id)?.clone();
    let wb = document.source_wb(id);
    let file = document.file_key(&path);
    Ok(source::Request {
        reference: exposures.map_or_else(
            || {
                Arc::new(
                    document
                        .reference_exposure(&expected)
                        .cloned()
                        .map_or_else(std::sync::OnceLock::new, std::sync::OnceLock::from),
                )
            },
            |store| document.exposure_slot(store, id),
        ),
        input: document.project.input,
        path,
        expected: Some(expected),
        wb,
        sources,
        file,
        correction,
    })
}

#[derive(Clone)]
pub(super) struct PreparationServices(pub source::Services);

impl PreparationServices {
    pub async fn prepare(
        self,
        mut request: Request,
        cancelled: crate::cancellation::Cancellation,
        progress: impl FnMut(usize, usize) + Send + 'static,
    ) -> Result<Prepared, String> {
        let sources = request.sources.take();
        let exposures = request.exposures.clone();
        let device = self.0.device.clone();
        let retired = cancelled.clone();
        let Candidate {
            mut prepared,
            source,
        } = crate::background::run(move |_| {
            if retired.is_cancelled() {
                return Err(Error::UnusableSample("document preparation retired"));
            }
            candidate(request, &device, progress)
        })
        .await
        .map_err(|_| "Document preparation worker panicked".to_owned())?
        .map_err(|error| error.to_string())?;
        if cancelled.is_cancelled() {
            return Err("Document preparation retired".into());
        }
        if let Some(mut source) = source {
            source.sources = sources;
            prepared.ready = Some(self.0.clone().prepare(source, cancelled).await?);
        }
        if let Some(id) = prepared.selected
            && let Some(ready) = &prepared.ready
            && let Some(reference) = &ready.reference
        {
            let metadata = Arc::make_mut(&mut prepared.document.exposure_metadata);
            metadata.entries.retain(|entry| entry.source != id);
            metadata.entries.push(crate::exposure_metadata::Entry {
                source: id,

                reference: reference.clone(),
            });
        }
        let device = self.0.device.clone();
        crate::background::run(move |_| {
            if let Some(exposures) = &exposures {
                prepared.document.capture_exposure(exposures);
            }
            prepared.document.validate_with_device(Some(&device))?;
            Ok(prepared)
        })
        .await
        .map_err(|_| "Document admission worker panicked".to_owned())?
        .map_err(|error: Error| error.to_string())
    }
}

#[derive(Default)]
pub(super) struct State {
    task: Option<async_executor::Task<()>>,
}

impl Editor {
    pub(super) fn start_document(
        &mut self,
        work: Preparation,
        cx: &crate::application::UiContext<'_, Self>,
    ) {
        if self.dust_search_pending() {
            self.status = "Wait for donor selection before changing the document".into();
            return;
        }

        self.cancel_spatial();
        self.cancel_control_edit();

        self.history_pending = match &work {
            Preparation::New | Preparation::Open(_) => Some(history::Pending::Reset),
            Preparation::Import(..) | Preparation::Input(..) => Some(history::Pending::Edit {
                before: Box::new(self.document.state.project.clone()),
                label: if matches!(&work, Preparation::Import(..)) {
                    "Import photos"
                } else {
                    "Input stage"
                },
                photo: None,
            }),
            Preparation::History(_, _, direction) => Some(history::Pending::Restore {
                direction: *direction,
                cleanup: self.dusting().then(|| self.dust_outlines()),
            }),
        };
        self.cancel_active_tool();
        self.document.visit.masks.cancel();
        let id =
            self.begin_document_operation(matches!(&work, Preparation::New | Preparation::Open(_)));
        let activity = self.activities.start(
            ActivityOptions::new(match &work {
                Preparation::Import(..) => "Importing RAWs…",
                Preparation::Open(_) => "Opening project…",
                Preparation::New => "Creating project…",
                Preparation::Input(..) => "Switching input stage…",
                Preparation::History(..) => "Restoring edit…",
            })
            .modal(),
        );
        self.new_project_pending = matches!(work, Preparation::New);
        let source = if matches!(&work, Preparation::Import(_, _, Some(_))) {
            self.document
                .visit
                .canvas
                .ready
                .as_ref()
                .map(|ready| (self.id, ready.clone()))
        } else {
            None
        };
        let cache = (!matches!(work, Preparation::New | Preparation::Open(_)))
            .then(|| self.document.cache.clone());
        let request = Request {
            exposures: cache.as_ref().map(|_| self.document.exposures.clone()),
            sources: self
                .source_services
                .as_ref()
                .map(|services| services.provider(cache)),
            work,
            source,
            correction: self.document.correction.clone(),
        };
        let source = self.source_services.clone();
        let progress = activity.reporter();
        let cancelled = crate::cancellation::Cancellation::default();
        let retire = cancelled.cancel_on_drop();
        let Some(task) = cx.spawn(move |cx| async move {
            let _retire = retire;
            let result = async {
                let source = source.ok_or_else(|| "Document services unavailable".to_owned())?;
                PreparationServices(source)
                    .prepare(request, cancelled, move |done, total| {
                        progress.progress(done, total)
                    })
                    .await
            };
            let result = result.await.map(Box::new);
            drop(activity);
            cx.update(|editor| {
                editor.document_prepared(id, result);
                editor.acquire_working(&cx);
            });
        }) else {
            self.document_prepared(id, Err("Application stopped".into()));
            return;
        };
        self.candidate_operation.task = Some(task);
    }
}

impl Editor {
    pub(super) fn document_prepared(&mut self, id: u64, result: Result<Box<Prepared>, String>) {
        if self.document_pending != Some(id) {
            return;
        }
        self.candidate_operation = State::default();

        let new_project = std::mem::take(&mut self.new_project_pending);
        let pending_history = self.history_pending.take();
        self.document_pending = None;

        match result {
            Ok(prepared) => {
                let input_changed = self.document.project.input.is_negative()
                    != prepared.document.project.input.is_negative();

                if self.reset_repairs_pending {
                    self.document.dust = dust::State::default();
                }
                self.reset_repairs_pending = false;
                let replacement = matches!(pending_history, Some(history::Pending::Reset));
                let same_visit = !replacement
                    && self.document.visit.current_source == prepared.selected
                    && self
                        .document
                        .same_visit(prepared.ready.as_ref(), prepared.reuse_ready);
                let layer_selection =
                    if !replacement && self.document.visit.current_source == prepared.selected {
                        self.document.visit.adjustment_layers.selection_only()
                    } else {
                        AdjustmentLayerEditor::default()
                    };
                self.document.visit.source = source::State::default();
                if replacement {
                    self.document = self.document.replacement(prepared.document);
                } else {
                    self.document.install(prepared.document);
                }
                self.document.correction = prepared.correction;
                if let Some(ready) = &prepared.ready {
                    self.document.cache_ready(ready);
                }
                self.remember_imported_look();
                self.document.visit.adjustment_layers = layer_selection;
                if new_project {
                    self.tab = 0;
                    self.preferences
                        .sections
                        .insert("roll-calibration".into(), true);
                }
                self.id = id;
                if !same_visit {
                    self.replace_photo_scope(self.id);
                }
                self.document.visit.current_source = prepared.selected;
                if !prepared.reuse_ready && !prepared.preserve_view {
                    self.document.roll_selection = RollSelection::default();
                }
                self.document.roll_selection.reconcile(
                    &self.document.state.project.order,
                    self.document.visit.current_source,
                );
                self.document.selected = prepared
                    .selected
                    .and_then(|id| {
                        self.document
                            .state
                            .project
                            .order
                            .iter()
                            .position(|candidate| *candidate == id)
                    })
                    .unwrap_or(0);
                self.refresh_adjustment_layer_selection();
                self.document.clear_crop_edit();
                self.document.visit.canvas.last_layout = None;
                self.document.visit.canvas.ready = prepared.ready;
                if !prepared.reuse_ready && !prepared.preserve_view {
                    self.document.visit.canvas.viewport.fit();
                    self.document.source = !self.document.project.input.ready();
                }
                if input_changed {
                    self.document.source = !self.document.project.input.ready();
                }
                self.document.visit.return_notice = None;
                self.document.export.draft = None;
                #[cfg(test)]
                {
                    self.document.export.summary = None;
                }
                self.invalidate_original();
                self.status = if !input_changed && self.document.state.project.active_flat_field().is_some()
                {
                    "Project ready. Flat-field changes keep Dmin/Dmax RGB; re-pick calibration for this setup."
                } else {
                    "Project ready"
                }
                .into();
                log::info!(request = id; "document_ready");
                self.history_prepared(pending_history);
                self.observe_gamma_pivot();
            }
            Err(error) => {
                log::warn!(
                    request = id,
                    error = error.as_str();
                    "document_failed"
                );
                self.history_failed(pending_history);
                self.resume_source_after_document();
                self.status = error.to_string();
                if self.document.visit.source.pending() {
                    self.document.visit.return_notice = Some(self.status.clone());
                }
                self.source_epoch
                    .store(self.id, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
}
