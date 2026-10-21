use crate::{
    Error,
    color::Balance,
    coverage::Coverage,
    crop::Crop,
    export::{ExrPrecision, Overwrite},
    geometry::{Composition, PhotoOrientation, RollOrientation},
    gpu::{ProcessingInput, ProcessingParameters, ProcessingSignature},
    local_adjustments::PhotoGradeSnapshot,
    platform::host,
    processing::{Calibration, PhotoSettings, RollSettings},
    raw::Rect,
    refinement::ViewRequest,
    rendering::OutputTarget,
    scan::Scan,
    viewer::{GpuImage, ImagePaint},
    viewport::{PickDrag, Viewport},
};
use glam::DVec3;
use std::{
    path::PathBuf,
    sync::{Arc, mpsc},
    time::Duration,
};

#[path = "editor_cache_limits.rs"]
mod cache_limits;
#[path = "editor_controls.rs"]
mod controls;
#[path = "editor_development.rs"]
mod development;
#[path = "editor_source.rs"]
mod source;
use source::Cache as LoadCache;
#[path = "editor_document.rs"]
mod document;
#[path = "editor_document_scope.rs"]
mod document_scope;
use document_scope::DocumentScope;
#[path = "editor_editing.rs"]
mod editing;
#[path = "editor_history.rs"]
mod history;
#[path = "editor_icons.rs"]
mod icons;
#[path = "editor_menu.rs"]
mod menu;
#[path = "editor_preferences.rs"]
mod preferences;
#[path = "editor_ui.rs"]
mod presentation;
use crate::project::{Development, Source, SourceId};
use document::Document;
#[path = "editor_files.rs"]
mod files;
use files::{FileAction, Transition};
#[path = "editor_feedback.rs"]
mod feedback;
use feedback::{Failure, FailureKind};
#[path = "editor_activity.rs"]
mod activity;
use activity::{Task, TaskActivity, TaskProgress, TaskRequest};
#[path = "editor_fullscreen.rs"]
mod fullscreen;
use fullscreen::FullscreenView;
#[path = "editor_thumbnails.rs"]
mod thumbnails;
use thumbnails::Thumbnails;
#[path = "editor_looks.rs"]
mod look_ui;
#[path = "editor_look_workflow.rs"]
mod look_workflow;
#[path = "editor_processing.rs"]
mod processing;
use processing::Grades;
#[path = "editor_adjustments.rs"]
mod adjustments;
use adjustments::Adjustments;
#[path = "editor_working_view.rs"]
mod working_view;
#[path = "editor_working_workflow.rs"]
mod working_workflow;
use working_view::WorkingView;
#[path = "editor_grade_scope.rs"]
mod grade_scope;
#[path = "editor_original_view.rs"]
mod original_view;
#[path = "editor_original_workflow.rs"]
mod original_workflow;
use original_view::{OriginalPurpose, OriginalView};
#[path = "editor_selected_preview.rs"]
mod selected_preview;
use selected_preview::{SelectionPaint, SelectionTiming};
#[path = "editor_scopes.rs"]
mod scopes;
use scopes::{Scopes, Slot as ScopeSlot};
#[path = "editor_analysis.rs"]
mod analysis;
#[path = "editor_analysis_review.rs"]
mod analysis_review;
#[path = "editor_analysis_services.rs"]
mod analysis_services;
#[path = "editor_analysis_workflow.rs"]
mod analysis_workflow;
#[path = "editor_dust.rs"]
mod dust;
#[path = "editor_dust_workflow.rs"]
mod dust_workflow;
#[path = "editor_flat_field.rs"]
mod flat;
#[path = "editor_reference_services.rs"]
mod reference_services;
#[path = "editor_reference_workflow.rs"]
mod reference_workflow;
#[path = "editor_support_workflow.rs"]
mod support_workflow;
#[cfg(test)]
use flat::FlatPreview;
use flat::{Correction, FlatEdit};
#[path = "editor_auto_crop.rs"]
mod auto_crop;
#[path = "editor_crop.rs"]
mod crop_editor;
use crop_editor::CropEdit;
#[path = "editor_masks.rs"]
mod mask_editor;
use mask_editor::MaskEdit;
#[cfg(test)]
#[path = "editor_cache_preferences_tests.rs"]
mod cache_preferences_tests;
#[path = "editor_canvas.rs"]
mod canvas;
#[path = "editor_document_services.rs"]
mod document_services;
#[path = "editor_document_workflow.rs"]
mod document_workflow;
use document_services::{Preparation, Prepared, Request as DocumentRequest};
#[path = "editor_export_services.rs"]
mod export_services;
#[cfg(test)]
#[path = "editor_export_tests.rs"]
mod export_tests;
#[path = "editor_export_workflow.rs"]
mod export_workflow;
#[path = "editor_frame.rs"]
mod frame;
#[cfg(all(test, feature = "ui-tests"))]
#[path = "editor_kittest.rs"]
mod kittest_tests;
#[path = "editor_measurement.rs"]
mod measurement;
#[path = "editor_memory.rs"]
mod memory;
#[path = "editor_photo_scope.rs"]
mod photo_scope;
#[path = "editor_save.rs"]
mod save;
#[path = "editor_source_workflow.rs"]
mod source_workflow;
#[cfg(test)]
use measurement::Measurement;
#[path = "editor_navigation.rs"]
mod navigation;
#[path = "editor_roll_selection.rs"]
mod roll_selection;
#[cfg(test)]
#[path = "editor_tests.rs"]
mod tests;
#[path = "editor_viewer.rs"]
mod viewer_ui;
use roll_selection::RollSelection;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Pick {
    Dmin,
    Dmax,
    Gamma,
    RollWb,
    Wb,
    Flat,
    Analysis,
}

struct PickReturn {
    path: PathBuf,
    selected: usize,
    adjustment: Option<(SourceId, adjustments::Target)>,
}

#[derive(Clone, Copy)]
struct PickView {
    source: bool,
    viewport: Viewport,
    anchored: Viewport,
}

#[derive(Clone)]
struct Ready {
    developed: Arc<crate::scan::DevelopedSource>,
    file: thumbnails::FileKey,
    thumbnail: Option<(thumbnails::FileKey, Arc<thumbnails::Proxy>)>,
    path: PathBuf,
    area: Rect,
    // None: not requested; Some(None): no local hint (or explicit roll-search reset).
    analysis_support: Option<Option<Rect>>,
    floor: f64,
    proxy: crate::viewer::GpuProxy,
    record: Source,
    development: Development,
}

#[derive(Clone, Copy)]
enum OutputFormat {
    Tiff(OutputTarget),
    Exr(ExrPrecision),
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn prepare_source(
    renderers: &crate::renderer::RendererCache,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    path: PathBuf,
    wb: Option<[f64; 4]>,
    expected: Option<&Source>,
    correction: Option<&Arc<Correction>>,
    basis: Option<&crate::project::CameraBasis>,
) -> Result<(Ready, Scan), Error> {
    prepare_source_cached(
        renderers, device, queue, path, wb, expected, correction, basis, None,
    )
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn prepare_source_cached(
    renderers: &crate::renderer::RendererCache,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    path: PathBuf,
    wb: Option<[f64; 4]>,
    expected: Option<&Source>,
    correction: Option<&Arc<Correction>>,
    basis: Option<&crate::project::CameraBasis>,
    cache: Option<&LoadCache>,
) -> Result<(Ready, Scan), Error> {
    prepare_source_scheduled(
        renderers,
        device,
        queue,
        path,
        wb,
        expected,
        correction,
        basis,
        cache,
        &development::Scheduler::new(),
        &|| false,
    )
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn prepare_source_scheduled(
    renderers: &crate::renderer::RendererCache,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    path: PathBuf,
    wb: Option<[f64; 4]>,
    expected: Option<&Source>,
    correction: Option<&Arc<Correction>>,
    basis: Option<&crate::project::CameraBasis>,
    cache: Option<&LoadCache>,
    developments: &development::Scheduler,
    cancelled: &dyn Fn() -> bool,
) -> Result<(Ready, Scan), Error> {
    let developed = source::Request {
        path,
        wb,
        expected: expected.cloned(),
        correction: correction.cloned(),
        basis: basis.cloned(),
        cache: cache.cloned(),
    }
    .load(developments, cancelled)?;
    let scan = developed.context.scan()?;
    let ready = developed.prepare_gpu(renderers, device, queue, cancelled)?;
    Ok((ready, scan))
}

pub struct Editor {
    started: bool,
    menus: menu::Menus,
    submissions: crate::platform::SubmissionGate,
    thumbnails: Thumbnails,
    scopes: Scopes,
    grades: Grades,
    working: WorkingView,
    original: OriginalView,
    dust_services: Option<dust_workflow::Services>,
    reset_repairs_pending: bool,
    viewport: Viewport,
    fullscreen: Option<FullscreenView>,
    // Running physical work outlives logical cancellation.
    workflow_jobs: crate::blocking_jobs::BlockingJobs,
    id: u64,
    pending_preview: Option<thumbnails::SelectedPreview>,
    selection_timing: Option<Arc<SelectionTiming>>,
    busy: bool,
    look_services: Option<look_workflow::Services>,
    loaded_looks: Vec<Arc<crate::looks::LookAsset>>,
    status: String,
    tasks: TaskActivity,
    failure: Option<Failure>,
    document_failure: Option<FailureKind>,
    return_notice: Option<String>,
    dmin: Option<DVec3>,
    dmax: Option<DVec3>,
    floor: f64,
    roll: RollSettings,
    photo: PhotoSettings,

    roll_orientation: RollOrientation,
    photo_orientation: PhotoOrientation,
    crop: Option<Crop>,

    auto_crop_requested: Option<auto_crop::Target>,
    auto_crop_margin: f64,

    last_layout: Option<(crate::viewport::CanvasLayout, f32)>,
    target: OutputTarget,
    source: bool,
    source_exposure: f64,
    pick: Option<Pick>,
    region: Option<Coverage>,
    pick_drag: Option<PickDrag>,
    tab: usize,
    preferences: preferences::Preferences,
    preference_store: Option<preferences::Store>,
    new_project_pending: bool,
    document: DocumentScope,
    candidate_operation: document_workflow::State,
    history_pending: Option<history::Pending>,
    history_controls: Option<history::Controls>,
    #[cfg(test)]
    history_snapshot_clones: usize,
    control_commands: Vec<editing::Command>,
    current_source: Option<SourceId>,
    next_request: u64,
    document_pending: Option<u64>,
    export_operation: export_workflow::State,
    export_services: Option<export_services::Services>,
    transition: Option<Transition>,
    after_save: Option<Transition>,
    project_overwrite: Option<(PathBuf, Option<Transition>)>,
    allow_close: bool,
    selected: usize,
    roll_selection: RollSelection,
    removal_pending: Option<Vec<SourceId>>,
    overwrite: Option<(PathBuf, OutputFormat)>,
    correction: Option<Arc<Correction>>,
    flat_edit: Option<FlatEdit>,
    developments: Option<development::Scheduler>,
    source_services: Option<source::Services>,
    source_epoch: Arc<std::sync::atomic::AtomicU64>,
}

impl Editor {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, initial: Option<PathBuf>) -> Self {
        let submissions = crate::platform::SubmissionGate::default();
        let renderers = Arc::new(crate::renderer::RendererCache::new(submissions.clone()));
        renderers.prewarm(device.clone(), queue.clone());
        let developments = development::Scheduler::new();
        let thumbnails = Thumbnails::new(
            device.clone(),
            queue.clone(),
            renderers.clone(),
            developments.clone(),
        );
        let source_epoch = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let scopes = Scopes::new(device);
        let grades = Grades::new(device.clone());
        let device = device.clone();
        let queue = queue.clone();
        let mut editor = Self::state();
        editor.submissions = submissions.clone();
        editor.working = WorkingView::new(device.clone());
        let jobs = editor.workflow_jobs.clone();
        editor.look_services = Some(look_workflow::Services::new(
            jobs.clone(),
            renderers.clone(),
            device.clone(),
            queue.clone(),
            submissions.clone(),
        ));
        editor.source_services = Some(source::Services {
            developments: developments.clone(),
            jobs: jobs.clone(),
            renderers: renderers.clone(),
            device: device.clone(),
            queue: queue.clone(),
        });
        let screening = Arc::new(crate::gpu::dust_screening::Cache::default());
        editor.dust_services = Some(dust_workflow::Services::new(
            jobs.clone(),
            device.clone(),
            queue.clone(),
            submissions.clone(),
            screening.clone(),
        ));
        editor.export_services = Some(export_services::Services::new(
            jobs,
            renderers,
            device,
            queue,
            submissions,
            screening,
        ));
        editor.developments = Some(developments);
        editor.source_epoch = source_epoch;
        editor.thumbnails = thumbnails;
        editor
            .thumbnails
            .bind_sources(editor.document.cache().clone());
        editor.preferences.cache_limits = Some(editor.thumbnails.limits);
        editor.scopes = scopes;
        editor.grades = grades;
        if let Some(path) = initial {
            if path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("arw"))
            {
                editor.add_raws(vec![path]);
                editor.new_project_pending = editor.document_pending.is_some();
            } else {
                editor.start_document(Preparation::Open(path));
            }
        }
        editor
    }

    fn state() -> Self {
        let document = Document::default();
        let mut thumbnails = Thumbnails::default();
        thumbnails.bind_sources(document.cache().clone());
        Self {
            submissions: crate::platform::SubmissionGate::default(),
            developments: None,
            source_services: None,
            source_epoch: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            thumbnails,
            scopes: Scopes::default(),
            grades: Grades::default(),
            working: WorkingView::default(),
            original: OriginalView::new(Arc::default()),
            dust_services: None,
            reset_repairs_pending: false,
            viewport: Viewport::default(),
            fullscreen: None,
            started: false,
            menus: menu::Menus::default(),
            workflow_jobs: crate::blocking_jobs::BlockingJobs::new(1),
            id: 0,
            pending_preview: None,
            selection_timing: None,
            busy: false,
            look_services: None,
            loaded_looks: Vec::new(),
            status: "Add scans to begin".into(),
            tasks: TaskActivity::default(),
            failure: None,
            document_failure: None,
            return_notice: None,
            dmin: None,
            dmax: None,
            floor: 0.,
            roll: RollSettings::new_grade(),
            photo: PhotoSettings::default(),

            roll_orientation: RollOrientation::default(),
            photo_orientation: PhotoOrientation::default(),
            crop: None,

            auto_crop_requested: None,
            auto_crop_margin: crate::auto_crop::DEFAULT_MARGIN_PERCENT,

            last_layout: None,
            target: OutputTarget::DisplayP3,
            source: true,
            source_exposure: 0.,
            pick: None,
            region: None,
            pick_drag: None,
            tab: 0,
            preferences: preferences::Preferences::default(),
            preference_store: None,
            new_project_pending: false,
            history_pending: None,
            history_controls: None,
            #[cfg(test)]
            history_snapshot_clones: 0,
            control_commands: Vec::new(),
            document: document.into(),
            candidate_operation: document_workflow::State::default(),
            current_source: None,
            next_request: 0,
            document_pending: None,
            export_operation: export_workflow::State::default(),
            export_services: None,
            transition: None,
            after_save: None,
            project_overwrite: None,
            allow_close: false,
            selected: 0,
            roll_selection: RollSelection::default(),
            removal_pending: None,
            overwrite: None,
            correction: None,
            flat_edit: None,
        }
    }

    pub fn restore_ui_preferences(&mut self) {
        match preferences::Store::open_default() {
            Ok((store, preferences)) => {
                self.apply_preferences(preferences);
                self.preference_store = Some(store);
                self.persist_preferences();
                if self.document_pending.is_none() || self.new_project_pending {
                    self.preferences
                        .sections
                        .insert("roll-calibration".into(), true);
                }
            }
            Err(error) => eprintln!("UI preferences not loaded (file left unchanged): {error}"),
        }
    }

    fn apply_preferences(&mut self, mut preferences: preferences::Preferences) {
        let limits = *preferences
            .cache_limits
            .get_or_insert(self.thumbnails.limits);
        self.thumbnails.set_limits(limits);
        self.scopes.visible = preferences.scopes_visible;
        self.scopes.height_ratio = preferences.scope_height_ratio;
        self.preferences = preferences;
    }

    fn persist_preferences(&mut self) {
        if let Some(store) = &mut self.preference_store
            && let Err(error) = store.save(&self.preferences)
        {
            self.status = format!("Unable to save application preferences: {error}");
            eprintln!("{}", self.status);
        }
    }

    fn cache_preferences(&mut self, ui: &mut egui::Ui) {
        use cache_limits::{MAX_MIB, MIB};
        ui.set_min_width(330.);
        let mut limits = self.thumbnails.limits;
        let mut changed = false;
        for (label, bytes) in [
            ("CPU cache", &mut limits.cpu_bytes),
            ("GPU cache", &mut limits.gpu_bytes),
        ] {
            let mut mib = *bytes / MIB;
            let response = presentation::labelled_row(
                ui,
                label,
                Some("Optional retained storage; not total application memory."),
                |ui, _| {
                    ui.add(
                        egui::DragValue::new(&mut mib)
                            .range(0..=MAX_MIB)
                            .speed(32.)
                            .update_while_editing(false)
                            .suffix(" MiB"),
                    )
                },
            )
            .inner;
            response.widget_info(|| {
                let mut info = egui::WidgetInfo::drag_value(response.enabled(), mib as f64);
                info.label = Some(label.into());
                info
            });
            let cancelled = controls::numeric_cancelled(ui, &response);
            if response.changed() && response.enabled() && !cancelled && *bytes != mib * MIB {
                *bytes = mib * MIB;
                changed = true;
            }
        }
        if presentation::TextAction::new("Reset cache limits")
            .show(ui)
            .clicked()
        {
            limits = self.thumbnails.default_limits();
            changed = true;
        }
        ui.label(
            "0 disables optional retention. Selected and visible work may exceed these limits.",
        );
        if changed {
            self.thumbnails.set_limits(limits);
            self.preferences.cache_limits = Some(limits);
            self.persist_preferences();
        }
    }

    fn calibration(&self) -> Option<Calibration> {
        Calibration {
            dmin: self.dmin?,
            dmax: self.dmax?,
            input_floor: self.floor,
        }
        .validate()
        .ok()
    }

    fn seed_thumbnail(&mut self) {
        self.thumbnails.bind_sources(self.document.cache().clone());
        let Some(ready) = &mut self.document.visit.ready else {
            return;
        };
        let Some((file, proxy)) = ready.thumbnail.take() else {
            return;
        };
        let Some(id) = self.current_source else {
            return;
        };
        let source = self
            .document
            .state
            .project
            .source(id)
            .expect("accepted source");
        // Flat-field editing displays a reference while retaining the working source ID.
        let path = self
            .document
            .state
            .source_path(id)
            .expect("accepted source locator");
        if document::same_path(&path, &ready.record.path)
            && source.development == ready.record.development
            && self.document.state.project.development.as_ref() == Some(&ready.development)
        {
            self.thumbnails
                .seed_at(source, &ready.development, &ready.path, file, proxy);
        }
    }

    fn sync_document(&mut self) {
        let project = &mut self.document.project;
        project.roll = self.roll;
        project.orientation = self.roll_orientation;
        project.processing_model = self.roll.response.processing_model();
        project.dmin = self.dmin.map(|value| value.to_array());
        project.dmax = self.dmax.map(|value| value.to_array());
        if project.input_floor.is_some() || self.dmin.is_some() || self.dmax.is_some() {
            project.input_floor = (self.floor > 0.).then_some(self.floor);
        }
        project.output.tiff = match self.target {
            OutputTarget::Srgb => crate::project::TiffTarget::Srgb,
            OutputTarget::DisplayP3 => crate::project::TiffTarget::DisplayP3,
        };
        if let Some(id) = self.current_source {
            let source = project
                .source_mut(id)
                .expect("active source belongs to document");
            source.photo = self.photo;
            source.orientation = self.photo_orientation;
            source.crop = self.crop;
        }
    }

    fn request_id(&mut self) -> u64 {
        self.next_request += 1;
        self.next_request
    }

    fn start_document(&mut self, work: Preparation) {
        // Foreground measurements and document work retain their existing exclusion;
        // candidate preparation itself does not replace the accepted photo visit.
        self.cancel_measurement();
        self.document.history.finish_group();
        self.history_pending = match &work {
            Preparation::New | Preparation::Open(_) => Some(history::Pending::Reset),
            Preparation::Import(..) | Preparation::Correction(..) => Some(history::Pending::Edit {
                before: Box::new(self.document.project.clone()),
                label: if matches!(&work, Preparation::Import(..)) {
                    "Import photos"
                } else {
                    "Correction"
                },
                photo: None,
            }),
            Preparation::History(_, _, direction) => Some(history::Pending::Restore {
                direction: *direction,
                cleanup: self
                    .document
                    .dust
                    .cleanup
                    .then_some(self.document.dust.show_masks),
            }),
        };
        let id =
            self.begin_document_operation(matches!(&work, Preparation::New | Preparation::Open(_)));
        self.tasks.begin(match &work {
            Preparation::Import(..) => Task::Import(id),
            Preparation::Open(_) => Task::Open(id),
            Preparation::New => Task::New(id),
            Preparation::Correction(..) => Task::Correction(id),
            Preparation::History(..) => Task::History(id),
        });
        self.document_failure = match &work {
            Preparation::Correction(..) => Some(FailureKind::FlatCorrection),
            _ => None,
        };
        self.new_project_pending = matches!(work, Preparation::New);
        let source = if matches!(&work, Preparation::Import(_, _, Some(_))) {
            self.document
                .visit
                .ready
                .as_ref()
                .map(|ready| (self.id, ready.clone()))
        } else {
            None
        };
        let correction = self.correction.clone();
        self.candidate_operation.request = Some(DocumentRequest {
            work,
            source,
            correction,
        });
    }

    pub(super) fn begin_document_operation(&mut self, replacement: bool) -> u64 {
        self.document.write = save::State::default();
        self.candidate_operation = document_workflow::State::default();
        if let Some(session) = &mut self.flat_edit {
            session.request = None;
            session.task = None;
        }
        self.retire_look();
        self.finish_auto_crop(false);
        self.finish_dust_cleanup();
        self.reset_repairs_pending = replacement;
        if self.document.analysis_preparing.take().is_some() {
            self.cancel_analysis();
        }
        if let Some(review) = self.document.analysis_review.take() {
            self.tab = review.return_tab;
            self.scopes.visible = review.return_scopes_visible;
            self.pick = None;
            self.region = None;
            self.document.pick_return = None;
            self.document.pick_view = None;
        }
        self.document.visit.support = Default::default();
        self.pending_preview = None;
        self.selection_timing = None;
        if self.document.analysis_pending.take().is_some() {
            self.cancel_analysis();
        }
        self.thumbnails.cancel();
        let id = self.request_id();
        self.source_epoch
            .store(id, std::sync::atomic::Ordering::Relaxed);
        self.original.invalidate();
        self.document_pending = Some(id);
        self.busy = true;
        self.status = "Preparing project…".into();
        id
    }

    fn add_raws(&mut self, paths: Vec<PathBuf>) {
        self.sync_document();
        let reuse = self
            .document
            .visit
            .ready
            .as_ref()
            .and(self.current_source)
            .map(|source| (self.id, source));
        self.start_document(Preparation::Import(self.document.capture(), paths, reuse));
    }

    fn load(&mut self, path: PathBuf) {
        self.load_source(path, true);
    }

    fn load_source(&mut self, path: PathBuf, persistent: bool) {
        self.sync_document();
        if let Err(error) = self.validate_document_candidate(&self.document) {
            self.status = format!("Source not changed: {error}");
            return;
        }
        self.finish_dust_cleanup();
        self.document.dust.release_gpu();
        let started = std::time::Instant::now();
        let selecting_photo = persistent && self.pick.is_none() && self.flat_edit.is_none();
        self.document.visit.support = Default::default();
        self.seed_thumbnail();
        self.pending_preview = None;
        self.selection_timing = None;
        if self.document.analysis_pending.take().is_some() {
            self.cancel_analysis();
        }
        self.thumbnails.cancel();
        self.document.visit.adjustments = Adjustments::default();
        self.document.visit.mask_edit = None;
        self.current_source = self.document.find_source(&path);
        if selecting_photo {
            self.roll_selection
                .reconcile(&self.document.project.order, self.current_source);
        }
        self.photo = self
            .current_source
            .and_then(|id| self.document.project.source(id).ok())
            .map(|source| source.photo)
            .unwrap_or_default();
        self.photo_orientation = self
            .current_source
            .and_then(|id| self.document.project.source(id).ok())
            .map(|source| source.orientation)
            .unwrap_or_default();
        self.crop = self
            .current_source
            .and_then(|id| self.document.project.source(id).ok())
            .and_then(|source| source.crop);
        self.document.visit.crop_edit = None;
        self.last_layout = None;
        let expected = self.current_source.map(|id| {
            Box::new(
                self.document
                    .project
                    .source(id)
                    .expect("registered source")
                    .clone(),
            )
        });
        let wb = self
            .document
            .project
            .development
            .as_ref()
            .map(|value| value.fixed_wb);
        self.original.invalidate();
        self.viewport.fit();
        self.overwrite = None;
        self.return_notice = None;
        self.document.pick_view = None;
        self.document.pick_return = None;
        self.id = self.request_id();
        self.replace_photo_scope(self.id);
        self.source_epoch
            .store(self.id, std::sync::atomic::Ordering::Relaxed);
        self.document.visit.ready = None;
        self.selection_timing =
            selecting_photo.then(|| Arc::new(SelectionTiming::new(self.id, started)));
        self.region = None;
        self.pick = None;
        self.source = self
            .document
            .auto_crop_edit
            .as_ref()
            .map_or(self.calibration().is_none(), |d| d.source_view);
        self.busy = true;
        self.status = "Developing full-resolution RAW…".into();
        self.tasks.begin(Task::Photo(self.id));
        let cache = LoadCache::new(&path, expected.as_deref(), wb, &self.document);
        self.document.visit.source.key = cache.key.clone();
        self.document.visit.source.request = Some(source::Request {
            path,
            wb,
            expected: expected.map(|source| *source),
            correction: self.correction.clone(),
            basis: self.document.project.camera_basis.clone(),
            cache: Some(cache),
        });
        if selecting_photo
            && self.pick.is_none()
            && self.flat_edit.is_none()
            && let Some(source) = self.current_source
        {
            match self.thumbnails.selected_preview(
                self.id,
                &self.document,
                source,
                self.correction.as_ref(),
            ) {
                Ok(preview) => {
                    println!(
                        "selected_switch request={} cache_hit={} boundary=admission elapsed_ms={:.6}",
                        self.id,
                        preview.is_some(),
                        started.elapsed().as_secs_f64() * 1000.
                    );
                    self.pending_preview = preview;
                }
                Err(error) => eprintln!(
                    "selected_preview request={} unavailable reason={error}",
                    self.id
                ),
            }
        }
    }

    fn finish_pick(&mut self) {
        self.cancel_measurement();
        if self.document.analysis_review.is_some() {
            self.status = "Analysis review cancelled; areas and grade unchanged".into();
            self.end_analysis_review();
            return;
        }
        self.document.visit.support = Default::default();
        if self.flat_edit.is_some() {
            self.finish_flat_edit();
            return;
        }
        let view = self.document.pick_view.take();
        let source = view.map_or(self.calibration().is_none(), |view| view.source);
        let returning = self.document.pick_return.is_some();
        self.pick = None;
        self.region = None;
        self.pick_drag = None;
        if let Some(context) = self.document.pick_return.take() {
            if self
                .document
                .visit
                .ready
                .as_ref()
                .is_none_or(|ready| ready.path != context.path)
            {
                let notice = self.status.clone();
                self.load(context.path);
                self.return_notice = Some(notice);
            }
            self.selected = context.selected;
            if let Some((source, target)) = context.adjustment
                && self.current_source == Some(source)
                && self.adjustment_target() != target
            {
                self.select_adjustment(target);
            }
        }
        if let Some(view) = view {
            if returning || self.viewport == view.anchored {
                self.viewport = view.viewport;
            } else if let Some((layout, pixels_per_point)) = self.last_layout {
                self.viewport
                    .reframe(layout, layout.composition, pixels_per_point);
            }
        }
        self.source = self.calibration().is_none() || source;
    }

    fn begin_pick(&mut self, kind: Pick) {
        self.finish_dust_cleanup();
        if self.pick.is_some() {
            self.finish_pick();
        }
        if self.busy
            || self.document.visit.crop_edit.is_some()
            || self.document.auto_crop_edit.is_some()
            || self.document.visit.mask_edit.is_some()
        {
            return;
        }
        self.thumbnails.cancel();
        let viewport = self.viewport;
        // Missing calibration forces source display, not an explicit inspection choice.
        let source = self.calibration().is_some() && self.source;
        self.pick = Some(kind);
        self.region = None;
        self.pick_drag = None;
        self.source = matches!(kind, Pick::Dmin | Pick::Dmax | Pick::Analysis);
        if self.source {
            self.document.pick_return =
                self.document.visit.ready.as_ref().map(|ready| PickReturn {
                    path: ready.path.clone(),
                    selected: self.selected,
                    adjustment: self
                        .current_source
                        .map(|source| (source, self.adjustment_target())),
                });
        }
        if let Some((layout, pixels_per_point)) = self.last_layout {
            let next = if kind == Pick::Analysis {
                Composition::new(
                    layout.composition.area(),
                    RollOrientation::default(),
                    PhotoOrientation::default(),
                )
                .expect("loaded source has a valid active area")
            } else if self.source {
                Composition::new(
                    layout.composition.area(),
                    self.roll_orientation,
                    self.photo_orientation,
                )
                .expect("loaded source has a valid active area")
            } else {
                layout.composition
            };
            if next != layout.composition {
                self.viewport.reframe(layout, next, pixels_per_point);
            }
        }
        self.document.pick_view = Some(PickView {
            source,
            viewport,
            anchored: self.viewport,
        });
        if kind == Pick::Analysis {
            if self.document.analysis_review.is_some() {
                self.refresh_review_region();
            } else {
                self.request_analysis_support();
            }
        }
    }

    fn pick_roll_source(&mut self, id: SourceId) {
        let Some(index) = self
            .document
            .project
            .order
            .iter()
            .position(|candidate| *candidate == id)
        else {
            return;
        };
        let path = self.document.source_path(id).expect("selected roll source");
        let kind = self.pick;
        let context = self.document.pick_return.take();
        let view = self.document.pick_view;
        if self
            .document
            .visit
            .ready
            .as_ref()
            .is_none_or(|ready| ready.path != path)
        {
            self.load(path);
        }
        self.selected = index;
        self.pick = kind;
        self.document.pick_return = context;
        self.document.pick_view = view;
        self.region = None;
        self.source = true;
    }

    fn apply_pick(&mut self) {
        if self.document.analysis_review.is_some() {
            self.confirm_analysis_review();
            return;
        }
        if self.pick == Some(Pick::Analysis) {
            self.apply_analysis_area();
            return;
        }
        if self.pick == Some(Pick::Flat) {
            self.apply_flat_edit();
            return;
        }
        if let (Some(kind), Some(region)) = (self.pick, self.region) {
            self.thumbnails.cancel();
            self.queue_measurement(kind, region);
        }
    }

    fn export(&mut self, path: PathBuf, format: OutputFormat, overwrite: Overwrite) {
        if self.document.auto_crop_edit.is_some() {
            self.status = "Apply or cancel auto-crop before exporting".into();
            return;
        }
        if self
            .document
            .path
            .as_ref()
            .is_some_and(|project| document::same_path(project, &path))
        {
            self.status = "Export destination is the current project document".into();
            self.report_failure(FailureKind::Export, self.status.clone());
            return;
        }
        self.sync_document();
        if let (Some(_), Some(ready)) = (self.calibration(), &self.document.visit.ready) {
            let recipe = (|| -> Result<crate::image_recipe::ImageRecipe, Error> {
                self.validate_document_candidate(&self.document)?;
                let id = self
                    .current_source
                    .ok_or_else(|| Error::Project("no accepted source for export".into()))?;
                self.document.project.image_recipe(id)
            })();
            let recipe = match recipe {
                Ok(recipe) => recipe,
                Err(error) => {
                    self.status = error.to_string();
                    return;
                }
            };
            debug_assert_eq!(recipe.composition.area(), ready.area);
            let input = self.source_context().expect("ready export source");
            let request = self.request_id();
            self.export_operation = export_workflow::State::new(
                request,
                export_services::Request {
                    input,
                    path,
                    format,
                    recipe,
                    overwrite,
                },
            );
            self.thumbnails.cancel();
            self.busy = true;
            self.status = "Exporting full-resolution source; preview pixels are not used…".into();
            self.tasks.begin(Task::Export(request));
        }
    }

    fn controls(&mut self, ui: &mut egui::Ui) -> (Option<ScopeSlot>, bool) {
        use controls::{ColorControl, Parameter};
        use presentation::SectionHeader;

        let reviewing = self.document.analysis_review.is_some();
        let previous_orientation = (self.roll_orientation, self.photo_orientation);
        presentation::restore_expansion(ui.ctx(), self.preferences.sections.clone());
        let mut analyze = false;
        let scope_slot = self.scope_panel(ui);
        ui.add_space(8.);

        let mut next_tab = self.tab;
        presentation::stage_tabs(ui, &mut next_tab);
        if next_tab != self.tab {
            ui.ctx().memory_mut(|memory| {
                if let Some(id) = memory.focused() {
                    memory.surrender_focus(id);
                }
            });
            ui.ctx().request_repaint();
        }

        if self.tab == 0 {
            analyze = self.roll_tool_shelf(ui);
        } else if self.tab == 1 {
            ui.add_enabled_ui(!reviewing, |ui| self.photo_tool_shelf(ui));
        }

        let editable = !self.busy
            && !reviewing
            && self.document.auto_crop_edit.is_none()
            && self.document.visit.mask_edit.is_none()
            && self.document.visit.crop_edit.is_none()
            && self.flat_edit.is_none()
            && self.calibration().is_some();
        let can_pick = !self.busy
            && !reviewing
            && self.document.auto_crop_edit.is_none()
            && self.document.visit.mask_edit.is_none()
            && self.document.visit.crop_edit.is_none()
            && self.flat_edit.is_none()
            && self.document.visit.ready.is_some();
        let mut start_pick = None;
        if self.tab == 1 {
            // A conditional indicator must not change the following inputs' auto-ID sequence.
            ui.push_id("adjustment-target-indicator", |ui| {
                self.adjustment_target_label(ui)
            });
        }
        egui::ScrollArea::vertical()
            .id_salt(("inspector", self.tab))
            .show(ui, |ui| {
                self.auto_crop_controls(ui);
                if self.tab == 0 {
                    ui.push_id("roll", |ui| {
                        ui.add_enabled_ui(!reviewing, |ui| self.flat_controls(ui));
                        let calibration_summary = match (self.dmin.is_some(), self.dmax.is_some()) {
                            (true, true) => "Dmin + Dmax measured",
                            (false, false) => "Dmin + Dmax not measured",
                            (false, true) => "Dmin not measured",
                            (true, false) => "Dmax not measured",
                        };
                        presentation::section(
                            ui,
                            "roll-calibration",
                            "Calibration",
                            SectionHeader {
                                summary: calibration_summary,
                                changed: false,
                                reset: None,
                                default_open: Some(true),
                            },
                            |ui| {
                                if controls::calibration(
                                    ui,
                                    "dmin",
                                    "Dmin (film base)",
                                    self.dmin,
                                    can_pick,
                                    "Measure an unexposed area of film base.",
                                ) {
                                    start_pick = Some(Pick::Dmin);
                                }
                                if controls::calibration(
                                    ui,
                                    "dmax",
                                    "Dmax",
                                    self.dmax,
                                    can_pick,
                                    "Measure the densest film reference.",
                                ) {
                                    start_pick = Some(Pick::Dmax);
                                }
                            },
                        );
                        ui.add_enabled_ui(editable, |ui| {
                            let inversion_changed = self.roll.gamma
                                != crate::processing::DEFAULT_GAMMA
                                || self.roll.balance != Balance::default();
                            let (_, reset) = presentation::section(
                                ui,
                                "roll-gamma-balance",
                                "Gamma & balance",
                                SectionHeader {
                                    summary: &format!(
                                        "Gamma {:.2} · Balance {}",
                                        self.roll.gamma,
                                        if self.roll.balance == Balance::default() {
                                            "neutral"
                                        } else {
                                            "adjusted"
                                        }
                                    ),
                                    changed: inversion_changed,
                                    reset: Some(inversion_changed),
                                    default_open: Some(true),
                                },
                                |ui| {
                                    controls::parameter(ui, Parameter::Gamma, &mut self.roll.gamma);
                                    if controls::color(
                                        ui,
                                        ColorControl::GammaBalance,
                                        &mut self.roll.balance,
                                        can_pick,
                                    ) {
                                        start_pick = Some(Pick::Gamma);
                                    }
                                },
                            );
                            if reset && !controls::defer_reset(ui, controls::Reset::GammaAndBalance) {
                                self.roll.gamma = crate::processing::DEFAULT_GAMMA;
                                self.roll.balance = Balance::default();
                            }
                            let response_summary = match self.roll.response {
                                crate::processing::Response::Original => "Classic curve".into(),
                                crate::processing::Response::Film { toe, shoulder } => {
                                    format!("Toe {toe:.2} · Shoulder {shoulder:.4}")
                                }
                            };
                            let (_, reset) = presentation::section(
                                ui,
                                "roll-response",
                                "Toe & shoulder",
                                SectionHeader {
                                    summary: &response_summary,
                                    changed: self.roll.response
                                        != crate::processing::Response::film(),
                                    reset: Some(
                                        !self.roll.response.is_original()
                                            && self.roll.response
                                                != crate::processing::Response::film(),
                                    ),
                                    default_open: Some(false),
                                },
                                |ui| match &mut self.roll.response {
                                    crate::processing::Response::Original => {
                                        ui.label("Classic curve · camera model 4");
                                        if presentation::TextAction::new("Use adjustable response")
                                            .help(
                                                "Start with toe 2.5 and shoulder off; keep other adjustments.",
                                            )
                                            .show(ui)
                                            .clicked()
                                        {
                                            self.control_command(editing::Command::Reset(
                                                editing::Scope::Roll,
                                                controls::Reset::Response,
                                            ));
                                        }
                                    }
                                    crate::processing::Response::Film { toe, shoulder } => {
                                        controls::parameter(ui, Parameter::Toe, toe);
                                        controls::parameter(ui, Parameter::Shoulder, shoulder);
                                        ui.label("Shoulder: 0 = off");
                                    }
                                },
                            );
                            if reset && !controls::defer_reset(ui, controls::Reset::Response) {
                                self.roll.response = crate::processing::Response::film();
                            }
                            if controls::white_balance(
                                ui,
                                "roll-white-balance",
                                &mut self.roll.wb,
                                can_pick,
                            ) {
                                start_pick = Some(Pick::RollWb);
                            }
                            let (_, reset) = presentation::section(
                                ui,
                                "roll-exposure",
                                "Exposure",
                                SectionHeader {
                                    summary: &format!("{:+.2} EV", self.roll.exposure),
                                    changed: self.roll.exposure != 0.,
                                    reset: Some(self.roll.exposure != 0.),
                                    default_open: Some(false),
                                },
                                |ui| {
                                    controls::parameter(
                                        ui,
                                        Parameter::Exposure,
                                        &mut self.roll.exposure,
                                    );
                                },
                            );
                            if reset && !controls::defer_reset(ui, controls::Reset::Exposure) {
                                self.roll.exposure = 0.;
                            }
                        });
                    });
                } else if self.tab == 1 {
                    ui.add_enabled_ui(!reviewing, |ui| {
                        self.crop_controls(ui);
                        self.dust_controls(ui);
                    });
                    if self.adjustment_controls(ui, editable, can_pick) {
                        start_pick = Some(Pick::Wb);
                    }
                } else {
                    self.look_controls(ui);
                    ui.add_enabled_ui(
                        !reviewing
                            && self.document.visit.crop_edit.is_none()
                            && self.document.auto_crop_edit.is_none()
                            && self.flat_edit.is_none()
                            && self.document.visit.mask_edit.is_none(),
                        |ui| {
                            let changed = self.target != OutputTarget::DisplayP3;
                            let (_, reset) = presentation::section(
                                ui,
                                "rendering-target",
                                "Target",
                                SectionHeader {
                                    summary: if self.target == OutputTarget::DisplayP3 {
                                        "Display P3 · 100 nit"
                                    } else {
                                        "sRGB · 100 nit"
                                    },
                                    changed,
                                    reset: Some(changed),
                                    default_open: Some(true),
                                },
                                |ui| {
                                    presentation::inspector_dropdown(
                                        ui,
                                        "rendering-target",
                                        "Target",
                                        if self.target == OutputTarget::DisplayP3 {
                                            "Display P3"
                                        } else {
                                            "sRGB"
                                        },
                                        |ui| {
                                            ui.selectable_value(
                                                &mut self.target,
                                                OutputTarget::DisplayP3,
                                                "Display P3",
                                            );
                                            ui.selectable_value(
                                                &mut self.target,
                                                OutputTarget::Srgb,
                                                "sRGB",
                                            );
                                        },
                                    )
                                    .response
                                    .on_hover_text("Output colour space, not the monitor profile.");
                                },
                            );
                            if reset && !controls::defer_reset(ui, controls::Reset::Target) {
                                self.target = OutputTarget::DisplayP3;
                            }
                        },
                    );
                }
            });

        self.preferences.sections = presentation::collect_expansion(ui.ctx());
        if !reviewing {
            self.preferences.scopes_visible = self.scopes.visible;
        }
        self.preferences.scope_height_ratio = self.scopes.height_ratio;
        if previous_orientation != (self.roll_orientation, self.photo_orientation) {
            self.viewport.fit();
        }
        if let Some(kind) = start_pick {
            self.control_command(editing::Command::Pick(kind));
        }

        // Render outgoing numeric controls once to commit focus loss before switching stages.
        self.tab = next_tab;
        (scope_slot, analyze)
    }

    fn export_dialog(&mut self, format: OutputFormat) {
        let Some(ready) = &self.document.visit.ready else {
            return;
        };
        let extension = if matches!(format, OutputFormat::Tiff(_)) {
            "tif"
        } else {
            "exr"
        };
        let filename = format!(
            "{}.{}",
            ready.path.file_stem().unwrap_or_default().to_string_lossy(),
            extension
        );
        if let Some(path) = crate::platform::menu::with_native_dialog(|| {
            rfd::FileDialog::new()
                .set_title("Export Photo")
                .set_file_name(filename)
                .add_filter("Output", &[extension])
                .save_file()
        }) {
            if path.exists() {
                self.overwrite = Some((path, format));
            } else {
                self.export(path, format, Overwrite::Refuse);
            }
        }
    }
}

impl host::App for Editor {
    fn menu_state(&self) -> Option<crate::platform::menu::State> {
        Some(self.application_menu_state())
    }

    fn menu_action(&mut self, action: crate::platform::menu::Action) {
        self.menus.pending.push_back(action);
    }

    fn menu_paste_requested(&self) -> bool {
        self.application_menu_state()
            .enabled(crate::platform::menu::Action::Paste)
            && self
                .menus
                .pending
                .iter()
                .take_while(|action| self.menu_targets_text(**action))
                .any(|action| *action == crate::platform::menu::Action::Paste)
    }

    fn menu_input(&mut self, clipboard: Option<String>) -> Vec<egui::Event> {
        Editor::menu_input(self, clipboard)
    }

    fn native_menu_installed(&mut self) {
        self.menus.native = true;
    }

    fn quit_state(&self) -> Option<host::QuitState> {
        Some(self.application_quit_state())
    }

    fn request_quit(&mut self) {
        self.request_application_quit();
    }

    fn photo_fullscreen(&self) -> bool {
        self.fullscreen_requested()
    }

    fn photo_fullscreen_changed(&mut self, active: bool) {
        self.fullscreen_changed(active);
    }

    fn submission_gate(&self) -> crate::platform::SubmissionGate {
        self.submissions.clone()
    }

    fn processing_revision(&self) -> Option<(u64, u64)> {
        Some((
            self.id,
            self.working
                .revision()
                .unwrap_or_else(|| self.original.revision()),
        ))
    }

    fn on_exit(&mut self) {
        self.working.clear();
        if let Some(developments) = &self.developments {
            developments.close();
        }
        self.document.cache().close();
        self.thumbnails.cancel();
        if let Some(store) = &mut self.preference_store
            && let Err(error) = store.save(&self.preferences)
        {
            eprintln!("UI preferences not saved: {error}");
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, access: &crate::application::UiAccess<'_, Self>) {
        self.draw_ui(ui);
        if self.allow_close {
            return;
        }
        self.start_measurement_workflow(access);
        self.start_save_workflow(access);
        self.start_document_workflow(access);
        self.start_source_workflow(access);
        self.start_original_workflow(access);
        self.start_working_workflow(access);
        self.start_reference_workflow(access);
        self.start_support_workflow(access);
        self.start_look_workflow(access);
        self.start_analysis_preparation(access);
        self.start_analysis_workflow(access);
        self.start_dust_workflow(access);
        self.start_export_workflow(access);
        self.start_embedded_workflow(access, ui.ctx());
        self.start_thumbnail_workflow(access);
    }
}

impl Editor {
    fn draw_ui(&mut self, ui: &mut egui::Ui) {
        if self.allow_close {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if self.initialize_ui(ui) {
            return;
        }
        // egui popups close on Escape without consuming it; preserve their input ownership.
        let popup_was_open = egui::Popup::is_any_open(ui.ctx());
        let panel_escape_available = !popup_was_open
            && !(self.menus.text_focused && ui.input(|input| input.key_pressed(egui::Key::Escape)));
        let tool_escape_available = !popup_was_open && self.menus.panel.is_none();
        self.receive_frame();
        let fullscreen_was_active = self.fullscreen.is_some();
        self.update_fullscreen(ui.ctx());
        let mut actions = self.application_actions(ui);
        self.frame_shortcuts(
            ui,
            &mut actions.file,
            fullscreen_was_active,
            tool_escape_available,
        );

        let previous_roll = self.roll;
        let previous_photo = self.photo;
        let was_dirty = self.document.dirty();
        self.disable_blocked_workspace(ui);
        let _menu_focus = self.workspace_chrome(ui, &mut actions);
        let (scope_slot, analyze) = self.edit_frame_controls(ui, previous_roll, previous_photo);
        if self.document.analysis_review.is_some() && self.document.analysis_pending.is_none() {
            self.stash_review_region();
        }

        let menu_zoom = match actions.menu {
            Some(crate::platform::menu::Action::ActualSize) => Some(1.),
            Some(crate::platform::menu::Action::ZoomIn) => Some(-1.25),
            Some(crate::platform::menu::Action::ZoomOut) => Some(-1. / 1.25),
            _ => None,
        };
        let zoom_action = self.viewer_bar(ui).or(menu_zoom);
        let fullscreen = self.fullscreen.is_some();
        presentation::viewer_surface(ui, fullscreen, |ui| {
            self.viewer_canvas(ui, zoom_action, fullscreen);
        });
        let text_focused = ui.ctx().text_edit_focused();
        ui.ctx()
            .data_mut(|data| data.insert_temp(ui.id().with("editor-text-focus"), text_focused));
        self.paint_scope(scope_slot);
        self.sync_document();
        if self.document.dirty() != was_dirty {
            ui.ctx().request_repaint();
        }

        // Deferred actions must see validated edits and committed numeric focus loss.
        self.application_panel(ui.ctx(), panel_escape_available);
        self.dispatch_frame_actions(ui, actions, analyze);
        self.frame_dialogs(ui, !popup_was_open);
        if self.allow_close {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.menus.text_focused = ui.ctx().text_edit_focused();
        self.menus.history_input_blocked =
            egui::Popup::is_any_open(ui.ctx()) || ui.input(|input| input.pointer.any_down());
    }
}
