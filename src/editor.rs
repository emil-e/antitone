use crate::{
    Error,
    application::UiContext,
    color::Balance,
    coverage::Coverage,
    crop::Crop,
    export::ExrPrecision,
    geometry::{Composition, PhotoOrientation, RollOrientation},
    gpu::{ProcessingInput, ProcessingParameters, ProcessingSignature},
    local_adjustments::PhotoGradeSnapshot,
    platform::host,
    processing::{Calibration, InputSettings, PhotoSettings, RollSettings},
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
use crate::project::{Source, SourceId};
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
#[path = "editor_spatial.rs"]
mod spatial;
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
#[path = "editor_export_batch.rs"]
mod export_batch;
#[path = "editor_export_dialog.rs"]
mod export_dialog;
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
#[cfg(test)]
use crate::export::Overwrite;
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
    proxy: crate::viewer::GpuProxy,
    record: Source,
}

#[cfg(test)]
impl Ready {
    fn development(&self) -> &crate::project::Development {
        &self.record.development.decode
    }
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
) -> Result<(Ready, Scan), Error> {
    prepare_source_cached(
        renderers, device, queue, path, wb, expected, correction, None,
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
    cache: Option<&LoadCache>,
    developments: &development::Scheduler,
    cancelled: &dyn Fn() -> bool,
) -> Result<(Ready, Scan), Error> {
    let developed = source::Request {
        path,
        wb,
        expected: expected.cloned(),
        correction: correction.cloned(),
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
    preview_rendered: Option<(
        u64,
        crate::gpu::ProcessingSignature,
        crate::frame_raster::FrameRaster,
        OutputTarget,
        Arc<crate::gpu::view::RenderedView>,
    )>,
    selection_timing: Option<Arc<SelectionTiming>>,
    busy: bool,
    look_services: Option<look_workflow::Services>,
    loaded_looks: Vec<Arc<crate::looks::LookAsset>>,
    status: String,
    tasks: TaskActivity,
    failure: Option<Failure>,
    document_failure: Option<FailureKind>,
    return_notice: Option<String>,
    input: InputSettings,
    input_switch: Option<InputSettings>,
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
    export_dialog: Option<export_batch::Draft>,
    export_summary: Option<export_batch::Summary>,
    correction: Option<Arc<Correction>>,
    flat_edit: Option<FlatEdit>,
    developments: Option<development::Scheduler>,
    source_services: Option<source::Services>,
    source_epoch: Arc<std::sync::atomic::AtomicU64>,
}

impl Editor {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
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
        let device = device.clone();
        let queue = queue.clone();
        let mut editor = Self::state();
        editor.input = editor.document.project.input;
        editor.submissions = submissions.clone();
        editor.initialize_viewer(&device, &queue);
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
            editor
                .source_services
                .clone()
                .expect("source services initialized"),
        ));
        editor.developments = Some(developments);
        editor.source_epoch = source_epoch;
        editor.thumbnails = thumbnails;
        editor
            .thumbnails
            .bind_sources(editor.document.cache().clone());
        editor.preferences.cache_limits = Some(editor.thumbnails.limits);
        editor.scopes = scopes;
        editor
    }

    fn initialize_viewer(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        self.grades = Grades::new(device.clone());
        self.working = WorkingView::new(device.clone(), queue.clone());
        // Preview, thumbnail and viewport rendering share one lazy kernel set; initialising it
        // here avoids a shader-compilation pause when the first provisional view is selected.
        if let Ok(kernels) = pollster::block_on(crate::gpu::view::ViewKernels::new(device)) {
            let _ = self.working.kernels.set(kernels);
        }
    }

    pub fn open_initial(&mut self, path: PathBuf, cx: &UiContext<'_, Self>) {
        if files::is_raw_path(&path) {
            self.add_raws(vec![path], cx);
            self.new_project_pending = self.document_pending.is_some();
        } else {
            self.start_document(Preparation::Open(path), cx);
        }
    }

    fn state() -> Self {
        let mut document = Document::default();
        let input = InputSettings::Negative {
            dmin: None,
            dmax: None,
            input_floor: crate::processing::CAMERA_INPUT_FLOOR,
            response: crate::processing::Response::film(),
            gamma: crate::processing::DEFAULT_GAMMA,
            balance: Balance::default(),
        };
        let _ = &mut document;
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
            preview_rendered: None,
            selection_timing: None,
            busy: false,
            look_services: None,
            loaded_looks: Vec::new(),
            status: "Add scans to begin".into(),
            tasks: TaskActivity::default(),
            failure: None,
            document_failure: None,
            return_notice: None,
            input,
            input_switch: None,
            roll: RollSettings::default(),
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
            export_dialog: None,
            export_summary: None,
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
            Err(error) => log::warn!(error = error.to_string().as_str(); "preferences_load_failed"),
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
            log::warn!(error = self.status.as_str(); "preferences_save_failed");
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

    #[cfg(test)]
    fn dmin(&self) -> Option<DVec3> {
        match self.input {
            InputSettings::Negative { dmin, .. } => dmin,
            InputSettings::Digital => None,
        }
    }

    #[cfg(test)]
    fn set_dmin(&mut self, value: Option<DVec3>) {
        if let InputSettings::Negative { dmin, .. } = self.negative_mut() {
            *dmin = value;
        }
        self.document.project.input = self.input;
    }

    #[cfg(test)]
    fn dmax(&self) -> Option<DVec3> {
        match self.input {
            InputSettings::Negative { dmax, .. } => dmax,
            InputSettings::Digital => None,
        }
    }

    #[cfg(test)]
    fn set_dmax(&mut self, value: Option<DVec3>) {
        if let InputSettings::Negative { dmax, .. } = self.negative_mut() {
            *dmax = value;
        }
        self.document.project.input = self.input;
    }

    #[cfg(test)]
    fn floor(&self) -> f64 {
        match self.input {
            InputSettings::Negative { input_floor, .. } => input_floor,
            InputSettings::Digital => 0.,
        }
    }

    #[cfg(test)]
    fn set_floor(&mut self, value: f64) {
        if let InputSettings::Negative { input_floor, .. } = self.negative_mut() {
            *input_floor = value;
        }
        self.document.project.input = self.input;
    }

    #[cfg(test)]
    fn balance(&self) -> Balance {
        match self.input {
            InputSettings::Negative { balance, .. } => balance,
            InputSettings::Digital => Balance::default(),
        }
    }

    #[cfg(test)]
    fn set_balance(&mut self, value: Balance) {
        if let InputSettings::Negative { balance, .. } = self.negative_mut() {
            *balance = value;
        }
        self.document.project.input = self.input;
    }

    #[cfg(test)]
    fn set_gamma(&mut self, value: f64) {
        if let InputSettings::Negative { gamma, .. } = self.negative_mut() {
            *gamma = value;
        }
        self.document.project.input = self.input;
    }

    #[cfg(test)]
    fn set_response(&mut self, value: crate::processing::Response) {
        if let InputSettings::Negative { response, .. } = self.negative_mut() {
            *response = value;
        }
        self.document.project.input = self.input;
    }

    #[cfg(test)]
    fn negative_mut(&mut self) -> &mut InputSettings {
        if !self.input.is_negative() {
            self.input = InputSettings::Negative {
                dmin: None,
                dmax: None,
                input_floor: crate::processing::CAMERA_INPUT_FLOOR,
                response: crate::processing::Response::film(),
                gamma: crate::processing::DEFAULT_GAMMA,
                balance: Balance::default(),
            };
        }
        &mut self.input
    }

    #[cfg(test)]
    fn response(&self) -> crate::processing::Response {
        match self.input {
            InputSettings::Negative { response, .. } => response,
            InputSettings::Digital => crate::processing::Response::Original,
        }
    }

    #[cfg(test)]
    fn gamma(&self) -> f64 {
        match self.input {
            InputSettings::Negative { gamma, .. } => gamma,
            InputSettings::Digital => crate::processing::DEFAULT_GAMMA,
        }
    }

    #[allow(dead_code)]
    fn calibration(&self) -> Option<Calibration> {
        self.input
            .calibration()
            .and_then(|calibration| calibration.validate().ok())
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
        {
            self.thumbnails
                .seed_at_path(source, &ready.path, file, proxy);
        }
    }

    fn input(&self) -> InputSettings {
        self.input
    }

    fn load_input(&mut self, input: InputSettings) {
        self.input = input;
    }

    fn negative_input(&self) -> InputSettings {
        InputSettings::Negative {
            dmin: None,
            dmax: None,
            input_floor: crate::processing::CAMERA_INPUT_FLOOR,
            response: crate::processing::Response::film(),
            gamma: crate::processing::DEFAULT_GAMMA,
            balance: Balance::default(),
        }
    }

    fn request_input(&mut self, target: InputSettings, cx: &UiContext<'_, Self>) {
        if target.is_negative() == self.input.is_negative() {
            return;
        }
        if self.input.has_stage_settings() && !target.is_negative() {
            self.input_switch = Some(target);
        } else {
            self.apply_input(target, cx);
        }
    }

    fn apply_input(&mut self, target: InputSettings, cx: &UiContext<'_, Self>) {
        if target.requires_scan_sources()
            && let Err(error) = self.document.project.require_scan_sources()
        {
            self.status = error.to_string();
            return;
        }
        if self.input.is_negative() != target.is_negative()
            && self
                .document
                .project
                .flat_field()
                .is_some_and(|field| field.enabled)
        {
            // The accepted map remains in the scan setup. Rebuild the live source context
            // through normal document preparation so no proxy retains the outgoing field.
            self.sync_document();
            let mut candidate = self.document.capture();
            candidate.project.input = target;
            self.start_document(Preparation::Input(candidate, self.current_source), cx);
            return;
        }
        if target.ready()
            && !self.document.project.sources.is_empty()
            && let Err(error) = self.validate_processing(target, self.roll, self.photo)
        {
            self.status = error.to_string();
            return;
        }
        let before = self.history_snapshot();
        self.input = target;
        self.source = !target.ready();
        self.record_edit(before, "Input stage", None);
    }

    fn input_stage_dialog(
        &mut self,
        context: &egui::Context,
        escape_available: bool,
        cx: &UiContext<'_, Self>,
    ) {
        let Some(target) = self.input_switch else {
            return;
        };
        let choice = presentation::DecisionDialog::new(
            "Switch input stage?",
            "Switching stages discards the current stage's settings: calibration, gamma, balance and response. Your source files are not changed.",
            "Switch stage",
        )
        .warning()
        .show(
            context,
            egui::Id::new("Switch input stage?"),
            escape_available,
        );
        match choice {
            Some(presentation::DialogChoice::Confirm) => {
                self.input_switch = None;
                self.apply_input(target, cx);
            }
            Some(presentation::DialogChoice::Cancel)
            | Some(presentation::DialogChoice::Alternative) => {
                self.input_switch = None;
            }
            None => {}
        }
    }

    fn sync_document(&mut self) {
        let input = self.input;
        let project = &mut self.document.project;
        project.input = input;
        project.roll = self.roll;
        project.orientation = self.roll_orientation;
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

    pub(super) fn begin_document_operation(&mut self, replacement: bool) -> u64 {
        self.document.write = save::State::default();
        self.candidate_operation = document_workflow::State::default();
        if let Some(session) = &mut self.flat_edit {
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
        self.preview_rendered = None;
        self.selection_timing = None;
        if self.document.analysis_pending.take().is_some() {
            self.cancel_analysis();
        }
        self.thumbnails.cancel();
        let id = self.request_id();
        self.source_epoch
            .store(id, std::sync::atomic::Ordering::Relaxed);
        self.invalidate_original();
        self.document_pending = Some(id);
        self.busy = true;
        self.status = "Preparing project…".into();
        id
    }

    fn add_raws(&mut self, paths: Vec<PathBuf>, cx: &UiContext<'_, Self>) {
        self.sync_document();
        let reuse = self
            .document
            .visit
            .ready
            .as_ref()
            .and(self.current_source)
            .map(|source| (self.id, source));
        self.start_document(
            Preparation::Import(self.document.capture(), paths, reuse),
            cx,
        );
    }

    fn load(&mut self, path: PathBuf, cx: &UiContext<'_, Self>) {
        self.load_source(path, true, cx);
    }

    fn finish_pick(&mut self, cx: &UiContext<'_, Self>) {
        self.cancel_measurement();
        if self.document.analysis_review.is_some() {
            self.status = "Analysis review cancelled; areas and grade unchanged".into();
            self.end_analysis_review(cx);
            return;
        }
        self.document.visit.support = Default::default();
        if self.flat_edit.is_some() {
            self.finish_flat_edit();
            return;
        }
        let view = self.document.pick_view.take();
        let source = view.map_or(!self.input.ready(), |view| view.source);
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
                self.load(context.path, cx);
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
        self.source = !self.input.ready() || source;
    }

    fn begin_pick(&mut self, kind: Pick, cx: &UiContext<'_, Self>) {
        self.finish_dust_cleanup();
        if self.pick.is_some() {
            self.finish_pick(cx);
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
        let source = self.input.ready() && self.source;
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
                self.request_analysis_support(cx);
            }
        }
    }

    fn pick_roll_source(&mut self, id: SourceId, cx: &UiContext<'_, Self>) {
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
            self.load(path, cx);
        }
        self.selected = index;
        self.pick = kind;
        self.document.pick_return = context;
        self.document.pick_view = view;
        self.region = None;
        self.source = true;
    }

    fn apply_pick(&mut self, cx: &UiContext<'_, Self>) {
        if self.document.analysis_review.is_some() {
            self.confirm_analysis_review(cx);
            return;
        }
        if self.pick == Some(Pick::Analysis) {
            self.apply_analysis_area(cx);
            return;
        }
        if self.pick == Some(Pick::Flat) {
            self.apply_flat_edit(cx);
            return;
        }
        if let (Some(kind), Some(region)) = (self.pick, self.region) {
            self.start_measurement(kind, region, cx);
        }
    }

    fn controls(
        &mut self,
        ui: &mut egui::Ui,
        cx: &UiContext<'_, Self>,
    ) -> (Option<ScopeSlot>, bool) {
        use controls::{ColorControl, Parameter};
        use presentation::SectionHeader;

        let reviewing = self.document.analysis_review.is_some();
        let initial_input = self.input;
        let previous_orientation = (self.roll_orientation, self.photo_orientation);
        presentation::inspector_spacing(ui);
        // The inspector column width is authoritative; content must not widen the panel.
        let inspector_width = ui.available_width();
        ui.set_width(inspector_width);
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
            analyze = self.roll_tool_shelf(ui, cx);
        } else if self.tab == 1 {
            ui.add_enabled_ui(!reviewing, |ui| self.photo_tool_shelf(ui, cx));
        }

        let editable = !self.busy
            && !reviewing
            && self.document.auto_crop_edit.is_none()
            && self.document.visit.mask_edit.is_none()
            && self.document.visit.crop_edit.is_none()
            && self.flat_edit.is_none()
            && self.input.ready();
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
                    let mut input = self.input;
                    let mut stage_target = None;
                    ui.push_id("roll", |ui| {
                        let capabilities = input.capabilities();
                        let scan = self.document.project.require_scan_sources();
                        let scan_reason = scan
                            .as_ref()
                            .err()
                            .map(ToString::to_string)
                            .unwrap_or_default();
                        ui.horizontal(|ui| {
                            ui.label("Input stage");
                            egui::ComboBox::from_id_salt("input-stage")
                                .selected_text(if input.is_negative() {
                                    "Negative"
                                } else {
                                    "Digital"
                                })
                                .show_ui(ui, |ui| {
                                    if input.is_negative() {
                                        let _ = ui.selectable_label(true, "Negative");
                                    } else {
                                        ui.add_enabled_ui(scan.is_ok(), |ui| {
                                            if ui.selectable_label(false, "Negative").clicked() {
                                                stage_target = Some(self.negative_input());
                                            }
                                        })
                                        .response
                                        .on_disabled_hover_text(scan_reason.clone());
                                    }
                                    if ui
                                        .selectable_label(!input.is_negative(), "Digital")
                                        .clicked()
                                        && input.is_negative()
                                    {
                                        stage_target = Some(InputSettings::Digital);
                                    }
                                })
                                .response
                                .on_hover_text(
                                    "How this roll's sources become a positive before grading.",
                                );
                        });
                        if capabilities.flat_field {
                            ui.add_enabled_ui(!reviewing, |ui| self.flat_controls(ui, cx));
                        }
                        if let InputSettings::Negative { dmin, dmax, .. } = input {
                            let calibration_summary = match (dmin.is_some(), dmax.is_some()) {
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
                                    tool: None,
                                    default_open: Some(true),
                                },
                                |ui| {
                                    if controls::calibration(
                                        ui,
                                        "dmin",
                                        "Dmin (film base)",
                                        dmin,
                                        can_pick,
                                        "Measure an unexposed area of film base.",
                                    ) {
                                        start_pick = Some(Pick::Dmin);
                                    }
                                    if controls::calibration(
                                        ui,
                                        "dmax",
                                        "Dmax",
                                        dmax,
                                        can_pick,
                                        "Measure the densest film reference.",
                                    ) {
                                        start_pick = Some(Pick::Dmax);
                                    }
                                },
                            );
                        }
                        ui.add_enabled_ui(editable, |ui| {
                            if let InputSettings::Negative {
                                response,
                                gamma,
                                balance,
                                ..
                            } = &mut input
                            {
                                let inversion_changed = *gamma != crate::processing::DEFAULT_GAMMA
                                    || *balance != Balance::default();
                                let (_, reset) = presentation::section(
                                    ui,
                                    "roll-gamma-balance",
                                    "Gamma & balance",
                                    SectionHeader {
                                        summary: &format!(
                                            "Gamma {gamma:.2} · Balance {}",
                                            if *balance == Balance::default() {
                                                "neutral"
                                            } else {
                                                "adjusted"
                                            }
                                        ),
                                        changed: inversion_changed,
                                        reset: Some(inversion_changed),
                                        tool: Some(controls::balance_pick_tool(can_pick, "Pick neutral for gamma balance")),
                                        default_open: Some(true),
                                    },
                                    |ui| {
                                        controls::parameter(ui, Parameter::Gamma, gamma);
                                        controls::balance_body(ui, ColorControl::GammaBalance, balance);
                                    },
                                );
                                if reset.tool { start_pick = Some(Pick::Gamma); }
                                if reset.reset
                                    && !controls::defer_reset(ui, controls::Reset::GammaAndBalance)
                                {
                                    *gamma = crate::processing::DEFAULT_GAMMA;
                                    *balance = Balance::default();
                                }
                                let response_summary = match response {
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
                                        changed: *response != crate::processing::Response::film(),
                                        reset: Some(
                                            !response.is_original()
                                                && *response != crate::processing::Response::film(),
                                        ),
                                        tool: None,
                                        default_open: Some(false),
                                    },
                                    |ui| match response {
                                        crate::processing::Response::Original => {
                                            ui.label("Classic curve · camera model 4");
                                            if presentation::TextAction::new(
                                                "Use adjustable response",
                                            )
                                            .help(
                                                "Start with toe 2.5 and shoulder off; keep other adjustments.",
                                            )
                                            .show(ui)
                                            .clicked()
                                            {
                                                self.control_command(
                                                    editing::Command::Reset(
                                                        editing::Scope::Roll,
                                                        controls::Reset::Response,
                                                    ),
                                                    cx,
                                                );
                                            }
                                        }
                                        crate::processing::Response::Film { toe, shoulder } => {
                                            controls::parameter(ui, Parameter::Toe, toe);
                                            controls::parameter(ui, Parameter::Shoulder, shoulder);
                                        }
                                    },
                                );
                                if reset.reset
                                    && !controls::defer_reset(ui, controls::Reset::Response)
                                {
                                    *response = crate::processing::Response::film();
                                }
                            }
                            if controls::white_balance(
                                ui,
                                "roll-white-balance",
                                &mut self.roll.wb,
                                can_pick,
                            ) {
                                start_pick = Some(Pick::RollWb);
                            }
                            let (_, action) = presentation::section(
                                ui,
                                "roll-exposure",
                                "Exposure",
                                SectionHeader {
                                    summary: &format!("{:+.2} EV", self.roll.exposure),
                                    changed: self.roll.exposure != 0.,
                                    reset: Some(self.roll.exposure != 0.),
                                    tool: None,
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
                            if action.reset && !controls::defer_reset(ui, controls::Reset::Exposure) {
                                self.roll.exposure = 0.;
                            }
                        });
                    });
                    // A reset command may already have replaced the staged input for this frame.
                    if self.input == initial_input {
                        self.input = input;
                    }
                    if let Some(target) = stage_target {
                        self.request_input(target, cx);
                    }
                } else if self.tab == 1 {
                    ui.add_enabled_ui(!reviewing, |ui| {
                        self.crop_controls(ui);
                        self.dust_controls(ui, cx);
                    });
                    if self.adjustment_controls(ui, cx, editable, can_pick) {
                        start_pick = Some(Pick::Wb);
                    }
                } else {
                    self.look_controls(ui, cx);
                    ui.add_enabled_ui(
                        !reviewing
                            && self.document.visit.crop_edit.is_none()
                            && self.document.auto_crop_edit.is_none()
                            && self.flat_edit.is_none()
                            && self.document.visit.mask_edit.is_none(),
                        |ui| {
                            let changed = self.target != OutputTarget::DisplayP3;
                            let (_, action) = presentation::section(
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
                                    tool: None,
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
                            if action.reset && !controls::defer_reset(ui, controls::Reset::Target) {
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
            self.control_command(editing::Command::Pick(kind), cx);
        }

        // Render outgoing numeric controls once to commit focus loss before switching stages.
        self.tab = next_tab;
        (scope_slot, analyze)
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
            log::warn!(error = error.to_string().as_str(); "preferences_save_failed");
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) {
        self.draw_ui(ui, cx);
    }
}

impl Editor {
    fn draw_ui(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) {
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
            cx,
            &mut actions.file,
            fullscreen_was_active,
            tool_escape_available,
        );

        let previous_settings = editing::ControlSettings::capture(self);
        let was_dirty = self.document.dirty();
        self.disable_blocked_workspace(ui);
        let _menu_focus = self.workspace_chrome(ui, cx, &mut actions);
        let (scope_slot, analyze) = self.edit_frame_controls(ui, cx, previous_settings);
        if self.document.analysis_review.is_some() && self.document.analysis_pending.is_none() {
            self.stash_review_region();
        }

        let menu_zoom = match actions.menu {
            Some(crate::platform::menu::Action::ActualSize) => Some(1.),
            Some(crate::platform::menu::Action::ZoomIn) => Some(-1.25),
            Some(crate::platform::menu::Action::ZoomOut) => Some(-1. / 1.25),
            _ => None,
        };
        let zoom_action = self.viewer_bar(ui, cx).or(menu_zoom);
        let fullscreen = self.fullscreen.is_some();
        presentation::viewer_surface(ui, fullscreen, |ui| {
            self.viewer_canvas(ui, zoom_action, fullscreen, cx);
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
        self.dispatch_frame_actions(ui, cx, actions, analyze);
        self.frame_dialogs(ui, cx, !popup_was_open);
        if self.allow_close {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.menus.text_focused = ui.ctx().text_edit_focused();
        self.menus.history_input_blocked =
            egui::Popup::is_any_open(ui.ctx()) || ui.input(|input| input.pointer.any_down());
    }
}
