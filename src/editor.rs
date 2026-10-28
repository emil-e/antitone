use crate::adjustment_layers::AdjustmentLayerSettings;
use crate::{
    Error,
    adjustment_layers::AdjustmentLayerSnapshot,
    application::UiContext,
    color::Balance,
    coverage::Coverage,
    crop::Crop,
    export::ExrPrecision,
    film_emulation::{FilmEmulationPreset, FilmEmulationSettings},
    geometry::{Composition, PhotoOrientation, RollOrientation},
    gpu::{ProcessingInput, ProcessingParameters, ProcessingSignature},
    platform::host,
    processing::{Calibration, InputSettings, RollSettings},
    raw::Rect,
    refinement::ViewRequest,
    rendering::OutputTarget,
    scan::Scan,
    viewer::{GpuImage, ImagePaint},
    viewport::{PickDrag, Viewport},
};
use glam::DVec3;
#[cfg(test)]
use std::sync::mpsc;
use std::{path::PathBuf, sync::Arc, time::Duration};

mod cache_limits;
mod controls;

use crate::source as source_loading;

mod source;

mod document;
mod document_scope;
use document_scope::DocumentScope;
mod editing;
mod history;
mod icons;
mod menu;
mod preferences;
mod presentation;
use crate::project::{Source, SourceId};
use document::Document;
mod files;
use files::{FileAction, Transition};
mod feedback;
use feedback::{Failure, FailureKind};
mod activity;
use activity::{Task, TaskActivity, TaskProgress, TaskRequest};
mod fullscreen;
use fullscreen::FullscreenView;
mod thumbnails;
use thumbnails::Thumbnails;
mod look_ui;
mod look_workflow;
mod processing;
use processing::Grades;
mod adjustment_layers;
use adjustment_layers::AdjustmentLayerEditor;
mod working_view;
mod working_workflow;
use working_view::WorkingView;
mod grade_scope;
mod original_view;
mod original_workflow;
use original_view::{OriginalPurpose, OriginalView};
mod selected_preview;
use selected_preview::{SelectionPaint, SelectionTiming};
mod scopes;
use scopes::{Scopes, Slot as ScopeSlot};
mod analysis;
mod analysis_review;
mod analysis_services;
mod analysis_workflow;
mod dust;
mod flat;
mod reference_services;
mod reference_workflow;
mod spatial;
mod support_workflow;
#[cfg(test)]
use flat::FlatPreview;
use flat::{Correction, FlatEdit};
mod auto_crop;
mod crop_editor;
use crop_editor::CropEdit;
mod mask_editor;
use mask_editor::MaskEdit;
#[cfg(test)]
mod cache_preferences_tests;
mod canvas;
mod document_services;
mod document_workflow;
use document_services::{Preparation, Prepared, Request as DocumentRequest};
#[cfg(all(test, feature = "raw", feature = "export"))]
mod ab_probe_tests;
mod export_batch;
mod export_dialog;
mod export_services;
#[cfg(test)]
mod export_tests;
mod export_workflow;
mod frame;
#[cfg(all(test, feature = "ui-tests"))]
mod kittest_tests;
mod measurement;
mod memory;
mod photo_scope;
mod picking;
mod save;
mod source_workflow;
#[cfg(test)]
use measurement::Measurement;
mod navigation;
#[cfg(test)]
use crate::export::Overwrite;
mod roll_selection;
#[cfg(test)]
mod tests;
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
    adjustment: Option<(SourceId, adjustment_layers::Target)>,
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
    reference: Option<crate::exposure_reference::ReferenceExposure>,
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
    cache: Option<(&Arc<crate::source::SourceCache>, thumbnails::FileKey)>,
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
        &source_loading::SourceLoader::new(),
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
    cache: Option<(&Arc<crate::source::SourceCache>, thumbnails::FileKey)>,
    developments: &source_loading::SourceLoader,
    cancelled: &dyn Fn() -> bool,
) -> Result<(Ready, Scan), Error> {
    let developed = source::Request {
        reference: Arc::default(),
        input: Default::default(),
        path: path.clone(),
        wb,
        expected: expected.cloned(),
        correction: correction.cloned(),
        file: cache
            .as_ref()
            .map_or_else(|| thumbnails::FileKey::new(&path), |(_, file)| file.clone()),
        sources: Some(developments.provider(cache.map(|(cache, _)| cache.clone()))),
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
    scopes: Scopes,
    reset_repairs_pending: bool,
    fullscreen: Option<FullscreenView>,
    // Running physical work outlives logical cancellation.
    id: u64,
    busy: bool,
    look_services: Option<look_workflow::Services>,
    loaded_looks: Vec<Arc<crate::looks::LookAsset>>,
    status: String,
    tasks: TaskActivity,
    failure: Option<Failure>,
    document_failure: Option<FailureKind>,

    auto_crop_margin: f64,

    source_exposure: f64,
    source_white_balance: crate::raw::SourceWhiteBalance,
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
    next_request: u64,
    document_pending: Option<u64>,
    export_services: Option<export_services::Services>,
    transition: Option<Transition>,
    after_save: Option<Transition>,
    project_overwrite: Option<(PathBuf, Option<Transition>)>,
    allow_close: bool,
    developments: Option<source_loading::SourceLoader>,
    source_services: Option<source::Services>,
    source_epoch: Arc<std::sync::atomic::AtomicU64>,
}

impl Editor {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let submissions = crate::platform::SubmissionGate::default();
        let renderers = Arc::new(crate::renderer::RendererCache::new(submissions.clone()));
        renderers.prewarm(device.clone(), queue.clone());
        let developments = source_loading::SourceLoader::new();
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
        editor.document.input = editor.document.state.project.input;
        editor.submissions = submissions.clone();
        editor.initialize_viewer(&device, &queue);
        editor.look_services = Some(look_workflow::Services::new(
            renderers.clone(),
            device.clone(),
            queue.clone(),
            submissions.clone(),
        ));
        editor.source_services = Some(source::Services {
            developments: developments.clone(),
            renderers: renderers.clone(),
            device: device.clone(),
            queue: queue.clone(),
        });
        editor.export_services = Some(export_services::Services::new(
            renderers,
            device,
            queue,
            submissions,
            editor
                .source_services
                .clone()
                .expect("source services initialized"),
        ));
        editor.developments = Some(developments);
        editor.source_epoch = source_epoch;
        editor.document.cache = thumbnails.sources.clone();
        editor.document.thumbnails = thumbnails;
        editor.preferences.cache_limits = Some(editor.document.thumbnails.limits);
        editor.scopes = scopes;
        editor
    }

    fn initialize_viewer(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        self.document.grades = Grades::new(device.clone());
        self.document.visit.working_view = WorkingView::new(device.clone(), queue.clone());
        // Preview, thumbnail and viewport rendering share one lazy kernel set; initialising it
        // here avoids a shader-compilation pause when the first provisional view is selected.
        if let Ok(kernels) = pollster::block_on(crate::gpu::view::ViewKernels::new(device)) {
            let _ = self.document.visit.working_view.kernels.set(kernels);
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
        let document = Document::default();
        let input = InputSettings::Negative {
            dmin: None,
            dmax: None,
            input_floor: crate::processing::CAMERA_INPUT_FLOOR,
            response: crate::processing::Response::film(),
            gamma: crate::processing::DEFAULT_GAMMA,
            balance: Balance::default(),
        };
        let mut document: document_scope::DocumentScope = document.into();
        document.input = input;
        Self {
            submissions: crate::platform::SubmissionGate::default(),
            developments: None,
            source_services: None,
            source_epoch: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            scopes: Scopes::default(),
            reset_repairs_pending: false,
            fullscreen: None,
            started: false,
            menus: menu::Menus::default(),
            id: 0,
            busy: false,
            look_services: None,
            loaded_looks: Vec::new(),
            status: "Add scans to begin".into(),
            tasks: TaskActivity::default(),
            failure: None,
            document_failure: None,

            auto_crop_margin: crate::auto_crop::DEFAULT_MARGIN_PERCENT,

            source_exposure: 0.,
            source_white_balance: Default::default(),
            tab: 0,
            preferences: preferences::Preferences::default(),
            preference_store: None,
            new_project_pending: false,
            history_pending: None,
            history_controls: None,
            #[cfg(test)]
            history_snapshot_clones: 0,
            control_commands: Vec::new(),
            document,
            candidate_operation: document_workflow::State::default(),
            next_request: 0,
            document_pending: None,
            export_services: None,
            transition: None,
            after_save: None,
            project_overwrite: None,
            allow_close: false,
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

    fn film_emulation_controls(&mut self, ui: &mut egui::Ui) {
        let settings = &mut self.document.film_emulation;
        let changed = *settings != FilmEmulationSettings::default();
        let summary = if settings.enabled {
            format!("{:.3}% · a {:.3}", settings.size, settings.amount)
        } else {
            "Off".to_owned()
        };
        let (_, action) = presentation::section(
            ui,
            "film-emulation",
            "Film Emulation",
            presentation::SectionHeader {
                summary: &summary,
                changed,
                reset: Some(changed),
                tool: None,
                default_open: Some(true),
            },
            |ui| {
                ui.checkbox(&mut settings.enabled, "Enable");
                controls::parameter(ui, controls::Parameter::FilmAmount, &mut settings.amount);
                controls::parameter(ui, controls::Parameter::FilmSize, &mut settings.size);
                controls::parameter(ui, controls::Parameter::FilmColour, &mut settings.colour);
                ui.horizontal(|ui| {
                    if ui.button("Fine").clicked() {
                        *settings = FilmEmulationPreset::Fine.settings();
                    }
                    if ui.button("Medium").clicked() {
                        *settings = FilmEmulationPreset::Medium.settings();
                    }
                    if ui.button("Coarse").clicked() {
                        *settings = FilmEmulationPreset::Coarse.settings();
                    }
                });
                ui.small("Source-anchored MTF and grain, before the creative look.");
            },
        );
        if action.reset {
            *settings = FilmEmulationSettings::default();
        }
    }

    fn apply_preferences(&mut self, mut preferences: preferences::Preferences) {
        let limits = *preferences
            .cache_limits
            .get_or_insert(self.document.thumbnails.limits);
        self.document.thumbnails.set_limits(limits);
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
        let mut limits = self.document.thumbnails.limits;
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
            limits = self.document.thumbnails.default_limits();
            changed = true;
        }
        ui.label(
            "0 disables optional retention. Selected and visible work may exceed these limits.",
        );
        if changed {
            self.document.thumbnails.set_limits(limits);
            self.preferences.cache_limits = Some(limits);
            self.persist_preferences();
        }
    }

    #[cfg(test)]
    fn dmin(&self) -> Option<DVec3> {
        match self.document.input {
            InputSettings::Negative { dmin, .. } => dmin,
            InputSettings::Digital => None,
        }
    }

    #[cfg(test)]
    fn set_dmin(&mut self, value: Option<DVec3>) {
        if let InputSettings::Negative { dmin, .. } = self.negative_mut() {
            *dmin = value;
        }
        self.document.state.project.input = self.document.input;
    }

    #[cfg(test)]
    fn dmax(&self) -> Option<DVec3> {
        match self.document.input {
            InputSettings::Negative { dmax, .. } => dmax,
            InputSettings::Digital => None,
        }
    }

    #[cfg(test)]
    fn set_dmax(&mut self, value: Option<DVec3>) {
        if let InputSettings::Negative { dmax, .. } = self.negative_mut() {
            *dmax = value;
        }
        self.document.state.project.input = self.document.input;
    }

    #[cfg(test)]
    fn floor(&self) -> f64 {
        match self.document.input {
            InputSettings::Negative { input_floor, .. } => input_floor,
            InputSettings::Digital => 0.,
        }
    }

    #[cfg(test)]
    fn set_floor(&mut self, value: f64) {
        if let InputSettings::Negative { input_floor, .. } = self.negative_mut() {
            *input_floor = value;
        }
        self.document.state.project.input = self.document.input;
    }

    #[cfg(test)]
    fn balance(&self) -> Balance {
        match self.document.input {
            InputSettings::Negative { balance, .. } => balance,
            InputSettings::Digital => Balance::default(),
        }
    }

    #[cfg(test)]
    fn set_balance(&mut self, value: Balance) {
        if let InputSettings::Negative { balance, .. } = self.negative_mut() {
            *balance = value;
        }
        self.document.state.project.input = self.document.input;
    }

    #[cfg(test)]
    fn set_gamma(&mut self, value: f64) {
        if let InputSettings::Negative { gamma, .. } = self.negative_mut() {
            *gamma = value;
        }
        self.document.state.project.input = self.document.input;
    }

    #[cfg(test)]
    fn set_response(&mut self, value: crate::processing::Response) {
        if let InputSettings::Negative { response, .. } = self.negative_mut() {
            *response = value;
        }
        self.document.state.project.input = self.document.input;
    }

    #[cfg(test)]
    fn negative_mut(&mut self) -> &mut InputSettings {
        if !self.document.input.is_negative() {
            self.document.input = InputSettings::Negative {
                dmin: None,
                dmax: None,
                input_floor: crate::processing::CAMERA_INPUT_FLOOR,
                response: crate::processing::Response::film(),
                gamma: crate::processing::DEFAULT_GAMMA,
                balance: Balance::default(),
            };
        }
        &mut self.document.input
    }

    #[cfg(test)]
    fn response(&self) -> crate::processing::Response {
        Self::response_of(self.document.input)
            .expect("negative fixture has an interpretation response")
    }

    #[cfg(test)]
    fn response_of(input: InputSettings) -> Option<crate::processing::Response> {
        match input {
            InputSettings::Negative { response, .. } => Some(response),
            InputSettings::Digital => None,
        }
    }

    #[cfg(test)]
    fn gamma(&self) -> f64 {
        match self.document.input {
            InputSettings::Negative { gamma, .. } => gamma,
            InputSettings::Digital => crate::processing::DEFAULT_GAMMA,
        }
    }

    #[allow(dead_code)]
    fn calibration(&self) -> Option<Calibration> {
        self.document
            .input
            .calibration()
            .and_then(|calibration| calibration.validate().ok())
    }

    fn seed_thumbnail(&mut self) {
        let Some(ready) = &mut self.document.visit.ready else {
            return;
        };
        let Some((file, proxy)) = ready.thumbnail.take() else {
            return;
        };
        let Some(id) = self.document.visit.current_source else {
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
            self.document
                .thumbnails
                .seed_at_path(source, &ready.path, file, proxy);
        }
    }

    fn input(&self) -> InputSettings {
        self.document.input
    }

    fn load_input(&mut self, input: InputSettings) {
        self.document.input = input;
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
        if target.is_negative() == self.document.input.is_negative() {
            return;
        }
        if self.document.input.has_stage_settings() && !target.is_negative() {
            self.document.input_switch = Some(target);
        } else {
            self.apply_input(target, cx);
        }
    }

    fn apply_input(&mut self, target: InputSettings, cx: &UiContext<'_, Self>) {
        if self.document.input.is_negative() != target.is_negative()
            && !self.document.state.project.sources.is_empty()
        {
            // Crossing stages changes every active source's decode WB, so it is an atomic
            // document operation rather than an input-only edit.
            self.sync_document();
            let mut candidate = self.document.capture();
            candidate.project.input = target;
            self.start_document(
                Preparation::Input(candidate, self.document.visit.current_source),
                cx,
            );
            return;
        }
        if target.ready()
            && !self.document.state.project.sources.is_empty()
            && let Err(error) = self.validate_processing(
                target,
                self.document.roll,
                self.document.visit.adjustment_layer_settings,
            )
        {
            self.status = error.to_string();
            return;
        }
        let before = self.history_snapshot();
        self.document.input = target;
        self.document.source = !target.ready();
        self.record_edit(before, "Input stage", None);
    }

    fn input_stage_dialog(
        &mut self,
        context: &egui::Context,
        escape_available: bool,
        cx: &UiContext<'_, Self>,
    ) {
        let Some(target) = self.document.input_switch else {
            return;
        };
        let choice = presentation::DecisionDialog::new(
            "Switch input stage?",
            "Switching stages discards the current stage's settings: calibration, gamma, balance, response and flat-field correction. Your source files are not changed.",
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
                self.document.input_switch = None;
                self.apply_input(target, cx);
            }
            Some(presentation::DialogChoice::Cancel)
            | Some(presentation::DialogChoice::Alternative) => {
                self.document.input_switch = None;
            }
            None => {}
        }
    }

    fn sync_document(&mut self) {
        let input = self.document.input;
        let adjustment_layer_target = self.adjustment_layer_target();
        let project = &mut self.document.state.project;
        project.input = input;
        project.roll = self.document.roll;
        project.orientation = self.document.roll_orientation;
        project.output.film_emulation = self.document.film_emulation;
        project.output.tiff = match self.document.target {
            OutputTarget::Srgb => crate::project::TiffTarget::Srgb,
            OutputTarget::DisplayP3 => crate::project::TiffTarget::DisplayP3,
        };
        if let Some(id) = self.document.visit.current_source {
            let source = project
                .source_mut(id)
                .expect("active source belongs to document");
            if let adjustment_layers::Target::Layer(layer) = adjustment_layer_target
                && let Some(record) = source
                    .adjustment_layers
                    .as_mut()
                    .and_then(|layers| layers.layers.iter_mut().find(|record| record.id == layer))
            {
                record.adjustment.photo = self.document.visit.adjustment_layer_settings;
            }
            source.orientation = self.document.visit.photo_orientation;
            source.crop = self.document.visit.crop;
        }
    }

    fn request_id(&mut self) -> u64 {
        self.next_request += 1;
        self.next_request
    }

    pub(super) fn begin_document_operation(&mut self, replacement: bool) -> u64 {
        self.document.write = save::State::default();
        self.candidate_operation = document_workflow::State::default();
        if let Some(session) = &mut self.document.flat_edit {
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
            self.document.picking.kind = None;
            self.document.picking.region = None;
            self.document.picking.return_to = None;
            self.document.picking.view = None;
        }
        self.document.visit.support = Default::default();
        self.document.visit.pending_preview = None;
        self.document.visit.preview_rendered = None;
        self.document.visit.selection_timing = None;
        if self.document.analysis_pending.take().is_some() {
            self.cancel_analysis();
        }
        self.document.thumbnails.cancel();
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
            .and(self.document.visit.current_source)
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
        if self.document.flat_edit.is_some() {
            self.finish_flat_edit();
            return;
        }
        let view = self.document.picking.view.take();
        let source = view.map_or(!self.document.input.ready(), |view| view.source);
        let returning = self.document.picking.return_to.is_some();
        self.document.picking.kind = None;
        self.document.picking.region = None;
        self.document.visit.pick_drag = None;
        if let Some(context) = self.document.picking.return_to.take() {
            if self
                .document
                .visit
                .ready
                .as_ref()
                .is_none_or(|ready| ready.path != context.path)
            {
                let notice = self.status.clone();
                self.load(context.path, cx);
                self.document.visit.return_notice = Some(notice);
            }
            self.document.selected = context.selected;
            if let Some((source, target)) = context.adjustment
                && self.document.visit.current_source == Some(source)
                && self.adjustment_layer_target() != target
            {
                self.select_adjustment_layer(target);
            }
        }
        if let Some(view) = view {
            if returning || self.document.visit.viewport == view.anchored {
                self.document.visit.viewport = view.viewport;
            } else if let Some((layout, pixels_per_point)) = self.document.visit.last_layout {
                self.document
                    .visit
                    .viewport
                    .reframe(layout, layout.composition, pixels_per_point);
            }
        }
        self.document.source = !self.document.input.ready() || source;
    }

    fn begin_pick(&mut self, kind: Pick, cx: &UiContext<'_, Self>) {
        self.finish_dust_cleanup();
        if self.document.picking.kind.is_some() {
            self.finish_pick(cx);
        }
        if self.busy
            || self.document.visit.crop_edit.is_some()
            || self.document.auto_crop_edit.is_some()
            || self.document.visit.mask_edit.is_some()
        {
            return;
        }
        self.document.thumbnails.cancel();
        let viewport = self.document.visit.viewport;
        // Missing calibration forces source display, not an explicit inspection choice.
        let source = self.document.input.ready() && self.document.source;
        self.document.picking.kind = Some(kind);
        self.document.picking.region = None;
        self.document.visit.pick_drag = None;
        self.document.source = matches!(kind, Pick::Dmin | Pick::Dmax | Pick::Analysis);
        if self.document.source {
            self.document.picking.return_to =
                self.document.visit.ready.as_ref().map(|ready| PickReturn {
                    path: ready.path.clone(),
                    selected: self.document.selected,
                    adjustment: self
                        .document
                        .visit
                        .current_source
                        .map(|source| (source, self.adjustment_layer_target())),
                });
        }
        if let Some((layout, pixels_per_point)) = self.document.visit.last_layout {
            let next = if kind == Pick::Analysis {
                Composition::new(
                    layout.composition.area(),
                    RollOrientation::default(),
                    PhotoOrientation::default(),
                )
                .expect("loaded source has a valid active area")
            } else if self.document.source {
                Composition::new(
                    layout.composition.area(),
                    self.document.roll_orientation,
                    self.document.visit.photo_orientation,
                )
                .expect("loaded source has a valid active area")
            } else {
                layout.composition
            };
            if next != layout.composition {
                self.document
                    .visit
                    .viewport
                    .reframe(layout, next, pixels_per_point);
            }
        }
        self.document.picking.view = Some(PickView {
            source,
            viewport,
            anchored: self.document.visit.viewport,
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
        let mut picking = std::mem::take(&mut self.document.picking);
        if self
            .document
            .visit
            .ready
            .as_ref()
            .is_none_or(|ready| ready.path != path)
        {
            self.load_source(path, false, cx);
        }
        self.document.selected = index;
        picking.region = None;
        self.document.picking = picking;
        self.document.source = true;
    }

    fn apply_pick(&mut self, cx: &UiContext<'_, Self>) {
        if self.document.analysis_review.is_some() {
            self.confirm_analysis_review(cx);
            return;
        }
        if self.document.picking.kind == Some(Pick::Analysis) {
            self.apply_analysis_area(cx);
            return;
        }
        if self.document.picking.kind == Some(Pick::Flat) {
            self.apply_flat_edit(cx);
            return;
        }
        if let (Some(kind), Some(region)) =
            (self.document.picking.kind, self.document.picking.region)
        {
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
        let initial_input = self.document.input;
        let previous_orientation = (
            self.document.roll_orientation,
            self.document.visit.photo_orientation,
        );
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
            && self.document.flat_edit.is_none()
            && self.document.input.ready();
        let can_pick = !self.busy
            && !reviewing
            && self.document.auto_crop_edit.is_none()
            && self.document.visit.mask_edit.is_none()
            && self.document.visit.crop_edit.is_none()
            && self.document.flat_edit.is_none()
            && self.document.visit.ready.is_some();
        let mut start_pick = None;

        egui::ScrollArea::vertical()
            .id_salt(("inspector", self.tab))
            .show(ui, |ui| {
                self.auto_crop_controls(ui);
                if self.tab == 0 {
                    let mut input = self.document.input;
                    let mut stage_target = None;
                    ui.push_id("roll", |ui| {
                        let capabilities = input.capabilities();
                        let scan = self.document.state.project.require_scan_sources();
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
                                        tool: Some(controls::balance_pick_tool(
                                            can_pick,
                                            "Pick neutral for gamma balance",
                                        )),
                                        default_open: Some(true),
                                    },
                                    |ui| {
                                        controls::parameter(ui, Parameter::Gamma, gamma);
                                        controls::balance_body(
                                            ui,
                                            ColorControl::GammaBalance,
                                            balance,
                                        );
                                    },
                                );
                                if reset.tool {
                                    start_pick = Some(Pick::Gamma);
                                }
                                if reset.reset
                                    && !controls::defer_reset(ui, controls::Reset::GammaAndBalance)
                                {
                                    *gamma = crate::processing::DEFAULT_GAMMA;
                                    *balance = Balance::default();
                                }
                                let response_summary = format!(
                                    "Toe {:.2} · Shoulder {:.4}",
                                    response.toe, response.shoulder
                                );
                                let (_, reset) = presentation::section(
                                    ui,
                                    "roll-response",
                                    "Toe & shoulder",
                                    SectionHeader {
                                        summary: &response_summary,
                                        changed: *response != crate::processing::Response::film(),
                                        reset: Some(
                                            *response != crate::processing::Response::film(),
                                        ),
                                        tool: None,
                                        default_open: Some(false),
                                    },
                                    |ui| {
                                        controls::parameter(ui, Parameter::Toe, &mut response.toe);
                                        controls::parameter(
                                            ui,
                                            Parameter::Shoulder,
                                            &mut response.shoulder,
                                        );
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
                                &mut self.document.roll.wb,
                                can_pick,
                            ) {
                                start_pick = Some(Pick::RollWb);
                            }
                            let (_, action) = presentation::section(
                                ui,
                                "roll-exposure",
                                "Exposure",
                                SectionHeader {
                                    summary: &format!("{:+.2} EV", self.document.roll.exposure),
                                    changed: self.document.roll.exposure != 0.,
                                    reset: Some(self.document.roll.exposure != 0.),
                                    tool: None,
                                    default_open: Some(false),
                                },
                                |ui| {
                                    controls::parameter(
                                        ui,
                                        Parameter::Exposure,
                                        &mut self.document.roll.exposure,
                                    );
                                },
                            );
                            if action.reset && !controls::defer_reset(ui, controls::Reset::Exposure)
                            {
                                self.document.roll.exposure = 0.;
                            }
                        });
                    });
                    // A reset command may already have replaced the staged input for this frame.
                    if self.document.input == initial_input {
                        self.document.input = input;
                    }
                    if let Some(target) = stage_target {
                        self.request_input(target, cx);
                    }
                } else if self.tab == 1 {
                    ui.add_enabled_ui(!reviewing, |ui| {
                        self.crop_controls(ui);
                        self.dust_controls(ui, cx);
                    });
                    if self.adjustment_layer_controls(ui, cx, editable, can_pick) {
                        start_pick = Some(Pick::Wb);
                    }
                } else {
                    if self.document.input.capabilities().film_emulation {
                        let allowed = !self.busy
                            && !reviewing
                            && self.document.visit.crop_edit.is_none()
                            && self.document.auto_crop_edit.is_none()
                            && self.document.flat_edit.is_none()
                            && self.document.visit.mask_edit.is_none();
                        ui.add_enabled_ui(allowed, |ui| self.film_emulation_controls(ui));
                    }
                    self.look_controls(ui, cx);
                    ui.add_enabled_ui(
                        !reviewing
                            && self.document.visit.crop_edit.is_none()
                            && self.document.auto_crop_edit.is_none()
                            && self.document.flat_edit.is_none()
                            && self.document.visit.mask_edit.is_none(),
                        |ui| {
                            let changed = self.document.target != OutputTarget::DisplayP3;
                            let (_, action) = presentation::section(
                                ui,
                                "rendering-target",
                                "Target",
                                SectionHeader {
                                    summary: if self.document.target == OutputTarget::DisplayP3 {
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
                                        if self.document.target == OutputTarget::DisplayP3 {
                                            "Display P3"
                                        } else {
                                            "sRGB"
                                        },
                                        |ui| {
                                            ui.selectable_value(
                                                &mut self.document.target,
                                                OutputTarget::DisplayP3,
                                                "Display P3",
                                            );
                                            ui.selectable_value(
                                                &mut self.document.target,
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
                                self.document.target = OutputTarget::DisplayP3;
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
        if previous_orientation
            != (
                self.document.roll_orientation,
                self.document.visit.photo_orientation,
            )
        {
            self.document.visit.viewport.fit();
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
            self.document
                .visit
                .working_view
                .revision()
                .unwrap_or_else(|| self.document.visit.grade.original.revision()),
        ))
    }

    fn on_exit(&mut self) {
        self.document.visit.working_view.clear();
        if let Some(developments) = &self.developments {
            developments.close();
        }
        self.document.thumbnails.cancel();
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
        self.workspace_chrome(ui, cx, &mut actions);
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
