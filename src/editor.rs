use crate::Error;
use crate::adjustment_layers::AdjustmentLayerSettings;
use crate::adjustment_layers::AdjustmentLayerSnapshot;
use crate::application::UiContext;
use crate::color::Balance;
use crate::coverage::Coverage;
use crate::crop::Crop;
#[cfg(test)]
use crate::export::ExrPrecision;
use crate::geometry::Composition;
use crate::geometry::PhotoOrientation;
use crate::geometry::RollOrientation;
#[cfg(test)]
use crate::gpu_export::authored::Format as OutputFormat;
use crate::photographic::stages::{
    grain::{self, GrainPreset, GrainSettings},
    halation,
};
use crate::platform::host;
use crate::processing::Calibration;
use crate::processing::InputSettings;
use crate::processing::RollSettings;
use crate::raw::Rect;
use crate::refinement::ViewRequest;
use crate::rendering::OutputTarget;
use crate::scan::Scan;
use crate::viewer::GpuImage;
use crate::viewer::{ImagePaint, Presentation};
use crate::viewport::PickDrag;
use crate::viewport::Viewport;
use glam::DVec3;
#[cfg(test)]
use std::sync::mpsc;
use std::{path::PathBuf, sync::Arc, time::Duration};

mod cache_limits;

use crate::source as source_loading;

mod source;

mod document;
mod document_scope;
use document_scope::DocumentScope;
mod editing;
mod history;
mod menu;
mod preferences;
mod tool;
mod ui;
use crate::project::{Source, SourceId};
use document::Document;
mod files;
use files::FileAction;
#[cfg(test)]
use files::Transition;
mod dialogs;

mod activity;
mod workspace_input;
#[cfg(test)]
use activity::Reporter as ActivityReporter;
use activity::{Activities, Activity, Options as ActivityOptions};
mod fullscreen;
use fullscreen::Fullscreen;
mod thumbnails;
use thumbnails::Thumbnails;
mod adjustment_layers;
pub mod external_edit;
pub mod integrations;
mod look;
mod processing;
use adjustment_layers::AdjustmentLayerEditor;
mod white_balance;
mod working_view;
use working_view::WorkingView;
mod original_view;
mod selected_preview;
use selected_preview::{SelectionPaint, SelectionTiming};
mod scopes;
use scopes::{Scopes, Slot as ScopeSlot};
mod analysis;

#[doc(hidden)]
pub fn check_semantic_wb_runtime() -> Result<std::path::PathBuf, crate::Error> {
    analysis::check_runtime()
}
#[cfg(test)]
use analysis::AnalyzeTool;
mod correction;
mod dust;
mod flat;
mod spatial;
use correction::Correction;
#[cfg(test)]
use flat::FlatEdit;
mod auto_crop;
#[cfg(test)]
mod cache_preferences_tests;
mod calibration;
mod canvas;
mod canvas_view;
mod crop_editor;
pub(crate) mod display;
mod export;
mod filmstrip;
mod gamma_pivot;
#[cfg(all(test, feature = "ui-tests"))]
mod kittest_tests;
mod lightbox;
mod mask_editor;
mod measurement;
mod memory;
mod navigation;
mod output;
mod photo_scope;
mod photo_ui;
mod picking;
mod save;
#[cfg(test)]
use crate::export::Overwrite;
#[cfg(test)]
mod input_tests;
mod roll_order;
mod roll_selection;
mod roll_ui;
#[cfg(test)]
mod tests;
mod viewer_ui;
use roll_selection::RollSelection;

#[cfg(test)]
use tests::{FailureCase, PickerCase};

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

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn prepare_source(
    renderers: &crate::editor::display::DisplayFixture,
    device: &wgpu::Device,
    queue: &crate::gpu::submission::Queue,
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
    renderers: &crate::editor::display::DisplayFixture,
    device: &wgpu::Device,
    queue: &crate::gpu::submission::Queue,
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
    renderers: &crate::editor::display::DisplayFixture,
    device: &wgpu::Device,
    queue: &crate::gpu::submission::Queue,
    path: PathBuf,
    wb: Option<[f64; 4]>,
    expected: Option<&Source>,
    correction: Option<&Arc<Correction>>,
    cache: Option<(&Arc<crate::source::SourceCache>, thumbnails::FileKey)>,
    developments: &source_loading::SourceLoader,
    cancelled: &dyn Fn() -> bool,
) -> Result<(Ready, Scan), Error> {
    let file = cache
        .as_ref()
        .map_or_else(|| thumbnails::FileKey::new(&path), |(_, file)| file.clone());
    let input = InputSettings::default();
    let physical = match expected {
        Some(source) => {
            if wb.is_some_and(|wb| wb != source.development.decode.white_balance) {
                return Err(Error::Project(
                    "source WB differs from accepted development".into(),
                ));
            }
            crate::source::Input::photo(
                path,
                file,
                input,
                Arc::default(),
                crate::source::input::Photograph {
                    record: source.clone(),
                    development: source.development.clone().into(),
                },
            )
        }
        None => crate::source::Input::reference(path, file, wb, input, Arc::default()),
    };
    let developed = source::Request {
        source: physical,
        correction: correction.cloned().into(),
        sources: Some(developments.provider(cache.map(|(cache, _)| cache.clone()))),
    }
    .load(developments, cancelled)?;
    let scan = developed.context.scan()?;
    renderers.evaluation().require_device(device)?;
    let ready = developed.prepare_gpu(
        renderers.evaluation().clone(),
        renderers.looks().clone(),
        &renderers.warmup(queue),
        cancelled,
    )?;
    Ok((ready, scan))
}

pub struct Editor {
    started: bool,
    menus: menu::Menus,
    scopes: Scopes,
    fullscreen: Option<Fullscreen>,
    canvas_frame: canvas::Frame,
    presentation: Result<Presentation, Error>,
    look: look::Session,
    status: String,
    workspace_input: workspace_input::WorkspaceInput,
    activities: Activities,
    #[cfg(test)]
    fixture_activity: Option<Activity>,
    dialogs: dialogs::Dialogs,

    auto_crop_margin: f64,

    source_exposure: f64,
    source_white_balance: crate::raw::SourceWhiteBalance,
    tab: usize,
    preferences: preferences::Preferences,
    preference_store: Option<preferences::Store>,
    memory_pressure: memory::Pressure,
    memory_watch: Option<async_executor::Task<()>>,
    document: DocumentScope,

    output_services: Option<output::Services>,
    allow_close: bool,
    developments: Option<source_loading::SourceLoader>,
    source_services: Option<source::Services>,
}

impl Editor {
    #[cfg(test)]
    fn submissions(&self) -> &crate::gpu::submission::Queue {
        self.document
            .visit
            .working_view
            .queue()
            .expect("initialized editor GPU")
    }

    fn interaction_blocked(&self) -> bool {
        self.activities.has_modal()
    }

    pub fn new(device: &wgpu::Device, queue: &crate::gpu::submission::Queue) -> Self {
        let cache_limits = cache_limits::Hardware::current(Some(device)).defaults();
        let evaluation = crate::evaluation::Context::new(device, Default::default());
        let looks = Arc::new(crate::photographic::stages::look::Looks::new(device));
        let display = display::Warmup::new(evaluation.clone(), queue.clone());
        display.start();
        let developments = source_loading::SourceLoader::with_gpu(device, queue);
        let thumbnails = Thumbnails::new(
            evaluation.clone(),
            queue.clone(),
            looks.clone(),
            display.clone(),
            developments.clone(),
        );
        let scopes = Scopes::new(device);
        let device = device.clone();
        let queue = queue.clone();
        let mut editor = Self::state();

        editor.initialize_viewer(&device, &queue);
        editor.look.services = Some(look::Services::new(device.limits()));
        editor.source_services = Some(source::Services {
            developments: developments.clone(),
            evaluation: evaluation.clone(),
            looks: looks.clone(),
            display: display.clone(),
            device: device.clone(),
            queue: queue.clone(),
        });
        editor.output_services = Some(output::Services::new(
            evaluation,
            looks,
            display,
            queue,
            editor
                .source_services
                .clone()
                .expect("source services initialized"),
        ));
        editor.developments = Some(developments);
        editor.document.cache = thumbnails.sources.clone();
        editor.document.thumbnails = thumbnails;
        editor.set_cache_limits(cache_limits);
        editor.scopes = scopes;
        editor
    }

    fn install_project(&mut self, document: Document) {
        self.cancel_active_tool();
        self.document = self.document.replacement(document);
        self.apply_cache_budgets();
        self.canvas_frame = Default::default();
        self.remember_imported_look();
        self.status = "Project ready".into();
    }

    fn activate_project(
        &mut self,
        document: Document,
        selected: Option<SourceId>,
        cx: &UiContext<'_, Self>,
    ) {
        self.install_project(document);
        if let Some(id) = selected.or_else(|| self.document.project.order.first().copied()) {
            self.browse_photo(id, cx);
        }
    }

    fn accept_external(&mut self, opened: external_edit::Opened, cx: &UiContext<'_, Self>) {
        let selected = opened.session.selected();
        self.activate_project(opened.document, Some(selected), cx);
        self.document.external_edit.session = Some(Arc::new(opened.session));
    }

    fn complete_external(&mut self) {
        if !self.document.dirty() && self.document_command_available() {
            self.allow_close = true;
        } else {
            self.status =
                "Snapshot returned. Resolve remaining edits or dialogs, then close this window."
                    .into();
        }
    }

    fn initialize_viewer(&mut self, device: &wgpu::Device, queue: &crate::gpu::submission::Queue) {
        self.presentation = pollster::block_on(Presentation::new(device));
        self.document.visit.working_view = WorkingView::new(device.clone(), queue.clone());
        // Preview, thumbnail and viewport rendering share one lazy kernel set; initialising it
        // here avoids a shader-compilation pause when the first provisional view is selected.
        if let Ok(kernels) = pollster::block_on(crate::viewer::view::ViewKernels::new(device)) {
            let _ = self.document.visit.working_view.kernels.set(kernels);
        }
    }

    pub fn open_initial(&mut self, path: PathBuf, cx: &UiContext<'_, Self>) {
        if files::is_raw_path(&path) {
            self.add_raws(vec![path], cx);
        } else {
            self.open_project(path, cx);
        }
    }

    fn state() -> Self {
        let document = Document::default();
        let input = InputSettings::Negative {
            dmin: None,
            dmax: None,
            response: crate::processing::Response::film(),
            gamma: crate::processing::DEFAULT_GAMMA,
            balance: Balance::default(),
        };
        let mut document: document_scope::DocumentScope = document.into();
        document.project.input = input;
        let workspace_input = workspace_input::WorkspaceInput::default();
        let mut preferences = preferences::Preferences::default();
        preferences.cache_limits = Some(cache_limits::Hardware::current(None).defaults());
        let mut editor = Self {
            developments: None,
            source_services: None,
            scopes: Scopes::default(),
            fullscreen: None,
            canvas_frame: Default::default(),
            presentation: Err(Error::Gpu("viewer presentation not initialized".into())),
            started: false,
            menus: menu::Menus::default(),
            look: Default::default(),
            status: "Add scans to begin".into(),
            activities: Activities::new(workspace_input.clone()),
            #[cfg(test)]
            fixture_activity: None,
            dialogs: dialogs::Dialogs::new(workspace_input.clone()),
            workspace_input,

            auto_crop_margin: crate::auto_crop::DEFAULT_MARGIN_PERCENT,

            source_exposure: 0.,
            source_white_balance: Default::default(),
            tab: 0,
            preferences,
            preference_store: None,
            #[cfg(not(test))]
            memory_pressure: crate::sys::memory_pressure(),
            #[cfg(test)]
            memory_pressure: memory::Pressure::Normal,
            memory_watch: None,

            document,
            output_services: None,
            allow_close: false,
        };
        editor.apply_cache_budgets();
        editor
    }

    pub fn restore_ui_preferences(&mut self) {
        match preferences::Store::open_default() {
            Ok((store, preferences)) => {
                self.apply_preferences(preferences);
                self.preference_store = Some(store);
                self.persist_preferences();
                if !self.document.files.opening() {
                    self.preferences
                        .sections
                        .insert("roll-calibration".into(), true);
                }
            }
            Err(error) => log::warn!(error = error.to_string().as_str(); "preferences_load_failed"),
        }
    }

    fn validate_roll_values(project: &crate::project::Project) -> Result<(), Error> {
        project.input.validate_controls()?;
        crate::processing::exposure_multiplier(project.roll.exposure)?;
        project.roll.wb.validate_roll_wb()?;
        Ok(())
    }

    fn roll_control(
        &mut self,
        response: ui::ControlResponse,
        cx: &UiContext<'_, Self>,
        write: impl FnOnce(&mut crate::project::Project),
    ) {
        self.gamma_control_response(response);
        if !response.changed && self.gamma_input_pending(response) {
            return;
        }
        let (starting_input, starting_wb) = self.gamma_control_initial(response);
        let Some(mut edit) = self.begin_control(response, false) else {
            return;
        };
        let accepted = self.document.state.project.clone();
        edit.update(write);
        let proposed = edit.preview().snapshot().unwrap().input;
        let gamma_changed = !gamma_pivot::same_gamma(accepted.input, proposed);
        if gamma_changed && self.gamma_compensation_required(starting_input, proposed) {
            drop(edit);
            self.compensate_gamma((starting_input, starting_wb), proposed, response, cx);
            return;
        }
        if gamma_changed || self.gamma_input_pending(response) {
            self.clear_gamma_proposal();
            if self.document.gamma_pivot.enabled() && proposed == starting_input {
                edit.update(|project| project.roll.wb = starting_wb);
            }
        }
        if let Err(error) = Self::validate_roll_values(&edit.preview().snapshot().unwrap()) {
            self.status = format!("Edit rejected: {error}");
            return;
        }
        self.commit_control(edit, response);
        if gamma_changed || !response.active {
            self.gamma_value_accepted(response);
        }
    }

    fn output_control(
        &mut self,
        response: ui::ControlResponse,
        write: impl FnOnce(&mut crate::project::Output),
    ) {
        let Some(mut edit) = self.begin_control(response, false) else {
            return;
        };
        let result = edit.update(|project| {
            write(&mut project.output);
            project.output.grain.validate()?;
            project.output.halation.validate()
        });
        if let Err(error) = result {
            self.status = format!("Edit rejected: {error}");
            return;
        }
        self.commit_control(edit, response);
    }

    fn grain_preset_controls(&mut self, ui: &mut egui::Ui) {
        let grade = self.document.project_snapshot();
        let current = [GrainPreset::ThirtyFiveMm, GrainPreset::MediumFormat]
            .into_iter()
            .find(|preset| preset.matches(grade.output.grain, grade.output.halation));
        let label = current.map_or("Custom", GrainPreset::label);
        let mut selected = None;
        ui::inspector_dropdown(ui, "grain-preset", "Preset", label, |ui| {
            for preset in [GrainPreset::ThirtyFiveMm, GrainPreset::MediumFormat] {
                if ui
                    .selectable_label(Some(preset) == current, preset.label())
                    .clicked()
                {
                    selected = Some(preset);
                }
            }
        });
        if let Some(preset) = selected {
            self.output_control(
                ui::ControlResponse::action(ui.make_persistent_id("grain-preset"), true),
                |output| {
                    output.grain = GrainSettings {
                        enabled: output.grain.enabled,
                        ..preset.settings()
                    };
                    output.halation = halation::HalationSettings {
                        enabled: output.halation.enabled,
                        ..preset.halation()
                    };
                },
            );
        }
    }

    fn halation_controls(&mut self, ui: &mut egui::Ui) {
        let mut settings = self.document.project_snapshot().output.halation;
        let changed = settings != halation::HalationSettings::default();
        let summary = if settings.active() {
            format!("{:.3}%", settings.size)
        } else {
            "Off".to_owned()
        };
        let (_, action) = ui::section(
            ui,
            "halation",
            "Halation",
            ui::SectionHeader {
                summary: &summary,
                changed,
                reset: Some(changed),
                tool: None,
                default_open: Some(true),
            },
            |ui| {
                let enabled = ui.checkbox(&mut settings.enabled, "Enable halation");
                self.output_control(ui::ControlResponse::discrete(&enabled), |output| {
                    output.halation.enabled = settings.enabled
                });
                let response = ui::ParameterRow::new(
                    "halation-strength",
                    "Strength",
                    0.0..=halation::MAX_STRENGTH,
                )
                .default_value(halation::DEFAULT_STRENGTH)
                .help("Redistribute positive light into a warm halo; 0 leaves the image unchanged.")
                .show(ui, &mut settings.strength)
                .edit;
                self.output_control(response, |output| {
                    output.halation.strength = settings.strength
                });
                let response = ui::ParameterRow::new(
                    "halation-size",
                    "Core size",
                    halation::MIN_SIZE..=halation::MAX_SIZE,
                )
                .default_value(halation::DEFAULT_SIZE)
                .slider_range(halation::MIN_SIZE..=0.21)
                .step(0.001)
                .decimals(3)
                .unit("%")
                .help("Core width as a percentage of the cropped frame's shorter side.")
                .slider_help("Slider: 0.01–0.21%; type larger sizes up to 0.50%.")
                .show(ui, &mut settings.size)
                .edit;
                self.output_control(response, |output| output.halation.size = settings.size);
            },
        );
        if action.reset {
            self.output_control(
                ui::ControlResponse::action(ui.make_persistent_id("reset-halation"), true),
                |output| output.halation = Default::default(),
            );
        }
    }

    fn grain_controls(&mut self, ui: &mut egui::Ui) {
        let mut settings = self.document.project_snapshot().output.grain;
        let changed = settings != GrainSettings::default();
        let summary = if settings.enabled {
            format!("{:.3}% · a {:.3}", settings.size, settings.amount)
        } else {
            "Off".to_owned()
        };
        let (_, action) = ui::section(
            ui,
            "grain",
            "Grain",
            ui::SectionHeader {
                summary: &summary,
                changed,
                reset: Some(changed),
                tool: None,
                default_open: Some(true),
            },
            |ui| {
                let enabled = ui.checkbox(&mut settings.enabled, "Enable");
                self.output_control(ui::ControlResponse::discrete(&enabled), |output| {
                    output.grain.enabled = settings.enabled
                });
                let response = ui::ParameterRow::new(
                    "grain-amount",
                    "Amount",
                    grain::MIN_AMOUNT..=grain::MAX_AMOUNT,
                )
                .default_value(grain::DEFAULT_AMOUNT)
                .step(0.001)
                .decimals(3)
                .help("Grain contrast before pixel averaging; 0 removes grain.")
                .show(ui, &mut settings.amount)
                .edit;
                self.output_control(response, |output| output.grain.amount = settings.amount);
                let response =
                    ui::ParameterRow::new("grain-size", "Size", grain::MIN_SIZE..=grain::MAX_SIZE)
                        .default_value(grain::DEFAULT_SIZE)
                        .step(0.001)
                        .decimals(3)
                        .unit("%")
                        .help(
                            "Scale grain and image softness together in fixed crop-relative units.",
                        )
                        .show(ui, &mut settings.size)
                        .edit;
                self.output_control(response, |output| {
                    output.grain = output.grain.resized(settings.size)
                });
                let response = ui::ParameterRow::new(
                    "grain-colour",
                    "Colour",
                    grain::MIN_COLOUR..=grain::MAX_COLOUR,
                )
                .default_value(grain::DEFAULT_COLOUR)
                .help(
                    "0 is monochrome; 0.5 matches the reference colour grain; 1 increases chroma.",
                )
                .show(ui, &mut settings.colour)
                .edit;
                self.output_control(response, |output| output.grain.colour = settings.colour);
            },
        );
        if action.reset {
            self.output_control(
                ui::ControlResponse::action(ui.make_persistent_id("reset-grain"), true),
                |output| output.grain = Default::default(),
            );
        }
    }

    fn apply_preferences(&mut self, mut preferences: preferences::Preferences) {
        let limits = *preferences.cache_limits.get_or_insert(self.cache_limits());
        self.set_cache_limits(limits);
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
        let mut limits = self.cache_limits();
        let mut changed = false;
        for (label, bytes) in [
            ("CPU cache", &mut limits.cpu_bytes),
            ("GPU cache", &mut limits.gpu_bytes),
        ] {
            let mut mib = *bytes / MIB;
            let response = ui::labelled_row(
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
            let cancelled = ui::numeric_cancelled(ui, &response);
            if response.changed() && response.enabled() && !cancelled && *bytes != mib * MIB {
                *bytes = mib * MIB;
                changed = true;
            }
        }
        if ui::TextAction::new("Reset cache limits").show(ui).clicked() {
            limits = cache_limits::Hardware::current(
                self.source_services
                    .as_ref()
                    .map(|services| &services.device),
            )
            .defaults();
            changed = true;
        }
        ui.label(
            "0 disables optional retention. Selected and visible work may exceed these limits.",
        );
        if changed {
            self.set_cache_limits(limits);
            self.persist_preferences();
        }
    }

    #[cfg(test)]
    fn dmin(&self) -> Option<DVec3> {
        match self.document.project.input {
            InputSettings::Negative { dmin, .. } => dmin,
            InputSettings::Digital => None,
        }
    }

    #[cfg(test)]
    fn set_dmin(&mut self, value: Option<DVec3>) {
        if let InputSettings::Negative { dmin, .. } = self.negative_mut() {
            *dmin = value;
        }
        self.document.state.project.input = self.document.project.input;
    }

    #[cfg(test)]
    fn dmax(&self) -> Option<DVec3> {
        match self.document.project.input {
            InputSettings::Negative { dmax, .. } => dmax,
            InputSettings::Digital => None,
        }
    }

    #[cfg(test)]
    fn set_dmax(&mut self, value: Option<DVec3>) {
        if let InputSettings::Negative { dmax, .. } = self.negative_mut() {
            *dmax = value;
        }
        self.document.state.project.input = self.document.project.input;
    }

    #[cfg(test)]
    fn balance(&self) -> Balance {
        match self.document.project.input {
            InputSettings::Negative { balance, .. } => balance,
            InputSettings::Digital => Balance::default(),
        }
    }

    #[cfg(test)]
    fn set_balance(&mut self, value: Balance) {
        if let InputSettings::Negative { balance, .. } = self.negative_mut() {
            *balance = value;
        }
        self.document.state.project.input = self.document.project.input;
    }

    #[cfg(test)]
    fn set_gamma(&mut self, value: f64) {
        if let InputSettings::Negative { gamma, .. } = self.negative_mut() {
            *gamma = value;
        }
        self.document.state.project.input = self.document.project.input;
    }

    #[cfg(test)]
    fn set_response(&mut self, value: crate::processing::Response) {
        if let InputSettings::Negative { response, .. } = self.negative_mut() {
            *response = value;
        }
        self.document.state.project.input = self.document.project.input;
    }

    #[cfg(test)]
    fn negative_mut(&mut self) -> &mut InputSettings {
        if !self.document.project.input.is_negative() {
            self.document.project.input = InputSettings::Negative {
                dmin: None,
                dmax: None,
                response: crate::processing::Response::film(),
                gamma: crate::processing::DEFAULT_GAMMA,
                balance: Balance::default(),
            };
        }
        &mut self.document.project.input
    }

    #[cfg(test)]
    fn response(&self) -> crate::processing::Response {
        Self::response_of(self.document.project.input)
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
        match self.document.project.input {
            InputSettings::Negative { gamma, .. } => gamma,
            InputSettings::Digital => crate::processing::DEFAULT_GAMMA,
        }
    }

    #[allow(dead_code)]
    fn calibration(&self) -> Option<Calibration> {
        self.document
            .project
            .input
            .calibration()
            .and_then(|calibration| calibration.validate().ok())
    }

    fn seed_thumbnail(&mut self) {
        let Some(ready) = &mut self.document.visit.canvas.ready else {
            return;
        };
        let Some((file, proxy)) = ready.thumbnail.take() else {
            return;
        };
        let Some(id) = self.document.visit.current_source else {
            return;
        };
        let Ok(source) = self.document.state.project.source(id) else {
            return;
        };
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
        self.document.project_snapshot().input
    }

    #[cfg(test)]
    fn load_input(&mut self, input: InputSettings) {
        self.document.project.input = input;
    }

    fn negative_input(&self) -> InputSettings {
        InputSettings::Negative {
            dmin: None,
            dmax: None,
            response: crate::processing::Response::film(),
            gamma: crate::processing::DEFAULT_GAMMA,
            balance: Balance::default(),
        }
    }

    fn request_input(&mut self, target: InputSettings, cx: &UiContext<'_, Self>) {
        if target.is_negative() == self.document.project.input.is_negative() {
            return;
        }
        let edit = match self.edit_project("Input stage") {
            Ok(edit) => edit,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        let discards_settings = !target.is_negative()
            && (self.document.project.input.has_stage_settings()
                || self.document.project.flat_field().is_some());
        if discards_settings {
            let response = self.dialogs.show(move |context, id, escape| {
                ui::DecisionDialog::new(
                    "Switch input stage?",
                    "Switching stages discards the current stage's settings: calibration, gamma, balance, response and flat-field correction. Your source files are not changed.",
                    "Switch stage",
                ).destructive().show(context, id, escape)
            });
            self.document.input_change = cx.spawn(move |cx| async move {
                let confirmed = response.await == Ok(ui::DialogChoice::Confirm);
                cx.update(|editor| {
                    editor.document.input_change = None;
                    if confirmed {
                        editor.publish_input(edit, target, &cx);
                    }
                });
            });
        } else {
            self.publish_input(edit, target, cx);
        }
    }

    #[cfg(test)]
    fn apply_input(&mut self, target: InputSettings, cx: &UiContext<'_, Self>) {
        match self.edit_project("Input stage") {
            Ok(edit) => self.publish_input(edit, target, cx),
            Err(error) => self.status = error.to_string(),
        }
    }

    fn publish_input(
        &mut self,
        mut edit: editing::ProjectEdit,
        target: InputSettings,
        cx: &UiContext<'_, Self>,
    ) {
        let stage_changed = self.document.project.input.is_negative() != target.is_negative();
        edit.update(|project| {
            project.input = target;
            if stage_changed {
                project.flat_field = None;
            }
        });
        if let Err(error) = self.commit_project(edit) {
            self.status = error.to_string();
            return;
        }
        self.document.source = !target.ready();
        if stage_changed {
            self.reinterpret_input(cx);
        }
    }

    fn reinterpret_input(&mut self, cx: &UiContext<'_, Self>) {
        self.document.source = !self.document.project.input.ready();
        self.document
            .state
            .capture_initialization(&self.document.initialization);
        self.cancel_spatial();
        self.document.hosted_tool = Default::default();
        self.reinterpret_photo();
        if let Some(id) = self.document.visit.current_source
            && let Ok(path) = self.document.source_path(id)
        {
            self.start_source_acquisition(path, cx);
        }
    }

    fn show_inspector(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) -> Option<ScopeSlot> {
        if self.fullscreen.is_some() {
            return None;
        }
        // The fixed name column and balance wheel set the useful minimum width.
        egui::Panel::right("controls")
            .default_size(360.)
            .min_size(270.)
            .show(ui, |ui| self.controls(ui, cx))
            .inner
    }

    fn controls(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) -> Option<ScopeSlot> {
        use ui::ColorControl;
        use ui::SectionHeader;

        let grade = self.document.project_snapshot();
        let requested_input = self.document.gamma_pivot.requested_input();
        let mut stage_target = None;
        let mut toggle_pivot = false;
        let previous_orientation = (self.document.project.orientation, self.photo_orientation());
        ui::inspector_spacing(ui);
        // The inspector column width is authoritative; content must not widen the panel.
        let inspector_width = ui.available_width();
        ui.set_width(inspector_width);
        ui::restore_expansion(ui.ctx(), self.preferences.sections.clone());
        let scope_slot = if self.showing_lightbox() {
            None
        } else {
            let slot = self.scope_panel(ui);
            ui.add_space(8.);
            slot
        };

        let mut next_tab = self.tab;
        ui::stage_tabs(ui, &mut next_tab);
        if next_tab != self.tab {
            ui.ctx().memory_mut(|memory| {
                if let Some(id) = memory.focused() {
                    memory.surrender_focus(id);
                }
            });
            ui.ctx().request_repaint();
        }

        if self.tab == 0 {
            roll_ui::tool_shelf(self, ui, cx);
        } else if self.tab == 1 {
            self.photo_tool_shelf(ui, cx);
        }

        let editable = !self.interaction_blocked();
        let can_pick = self.tool_start_available() && self.selected_area().is_some();
        let mut start_pick: Option<editing::Interaction> = None;

        egui::ScrollArea::vertical()
            .id_salt(("inspector", self.tab))
            .show(ui, |ui| {
                if self.document.hosted_tool.occupied() {
                    self.hosted_drawer(ui, cx);
                } else if self.tab == 1 && self.document.visit.hosted_tool.occupied() {
                    ui.add_enabled_ui(!self.interaction_blocked(), |ui| self.hosted_drawer(ui, cx));
                }
                if self.tab == 0 {
                    let input = grade.input;
                    ui.push_id("roll", |ui| {
                        let negative = input.is_negative();
                        let input_enabled = !self.tool_interacting();
                        let next = ui::input_selector(ui, negative, input_enabled);
                        if next != negative {
                            stage_target = Some(if next {
                                self.negative_input()
                            } else {
                                InputSettings::Digital
                            });
                        }

                        if negative {
                            if input.capabilities().flat_field {
                                self.flat_controls(ui, cx);
                            }
                            if let InputSettings::Negative { dmin, dmax, .. } = input {
                                let calibration_summary = match (dmin.is_some(), dmax.is_some()) {
                                    (true, true) => "Dmin + Dmax measured",
                                    (false, false) => "Dmin + Dmax not measured",
                                    (false, true) => "Dmin not measured",
                                    (true, false) => "Dmax not measured",
                                };
                                ui::section(
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
                                        if calibration::reference_control(
                                            ui,
                                            "dmin",
                                            "Dmin (film base)",
                                            dmin,
                                            can_pick,
                                            "Measure an unexposed area of film base.",
                                        ) {
                                            start_pick =
                                                Some(|cx| Box::pin(calibration::pick_dmin(cx)));
                                        }
                                        if calibration::reference_control(
                                            ui,
                                            "dmax",
                                            "Dmax",
                                            dmax,
                                            can_pick,
                                            "Measure the densest film reference.",
                                        ) {
                                            start_pick =
                                                Some(|cx| Box::pin(calibration::pick_dmax(cx)));
                                        }
                                    },
                                );
                            }
                            if let InputSettings::Negative {
                                mut response,
                                mut gamma,
                                mut balance,
                                dmin,
                                dmax,
                            } = requested_input.unwrap_or(grade.input)
                            {
                                let calibrated = dmin.is_some() && dmax.is_some();
                                let inversion_changed = gamma != crate::processing::DEFAULT_GAMMA
                                    || balance != Balance::default();
                                let (_, reset) = ui::section_with_secondary_tool(
                                    ui,
                                    "roll-gamma-balance",
                                    "Gamma & balance",
                                    SectionHeader {
                                        summary: &format!(
                                            "Gamma {gamma:.2} · Balance {}",
                                            if balance == Balance::default() {
                                                "neutral"
                                            } else {
                                                "adjusted"
                                            }
                                        ),
                                        changed: inversion_changed,
                                        reset: Some(inversion_changed && editable),
                                        tool: Some(ui::SectionTool {
                                            help: if calibrated {
                                                "Set gamma balance and Roll WB from one neutral patch."
                                            } else {
                                                "Measure Dmin and Dmax first."
                                            },
                                            ..ui::balance_pick_tool(
                                                can_pick && calibrated,
                                                calibration::GAMMA_PICK_LABEL,
                                            )
                                        }),
                                        default_open: Some(true),
                                    },
                                    self.gamma_pivot_tool(grade.input),
                                    |ui| {
                                        ui.add_enabled_ui(
                                            editable,
                                            |ui| {
                                                let response = ui::ParameterRow::new(
                                                    "gamma",
                                                    "Gamma",
                                                    crate::processing::MIN_GAMMA
                                                        ..=crate::processing::MAX_GAMMA,
                                                )
                                                .default_value(crate::processing::DEFAULT_GAMMA)
                                                .show(ui, &mut gamma)
                                                .edit;
                                                self.roll_control(response, cx, |project| {
                                                    if let InputSettings::Negative {
                                                        gamma: value,
                                                        ..
                                                    } = &mut project.input
                                                    {
                                                        *value = gamma;
                                                    }
                                                });
                                            },
                                        );
                                        ui.add_enabled_ui(
                                            editable,
                                            |ui| {
                                                for response in ui::BalanceControl::new(
                                                    "balance",
                                                    ColorControl::GammaBalance,
                                                )
                                                .show(ui, &mut balance)
                                                {
                                                    self.roll_control(response, cx, |project| {
                                                        if let InputSettings::Negative {
                                                            balance: value,
                                                            ..
                                                        } = &mut project.input
                                                        {
                                                            *value = balance;
                                                        }
                                                    });
                                                }
                                            },
                                        );
                                    },
                                );
                                if reset.tool {
                                    start_pick = Some(|cx| Box::pin(calibration::pick_gamma(cx)));
                                }
                                toggle_pivot |= reset.secondary_tool;
                                if reset.reset {
                                    self.roll_control(
                                        ui::ControlResponse::action(
                                            ui.make_persistent_id("reset-gamma-balance"),
                                            true,
                                        ),
                                        cx,
                                        |project| {
                                            if let InputSettings::Negative {
                                                gamma, balance, ..
                                            } = &mut project.input
                                            {
                                                *gamma = crate::processing::DEFAULT_GAMMA;
                                                *balance = Balance::default();
                                            }
                                        },
                                    );
                                }
                                let response_summary = format!(
                                    "Toe {:.2} · Shoulder {:.4}",
                                    response.toe, response.shoulder
                                );
                                let (_, reset) = ui::section(
                                    ui,
                                    "roll-response",
                                    "Toe & shoulder",
                                    SectionHeader {
                                        summary: &response_summary,
                                        changed: response != crate::processing::Response::film(),
                                        reset: Some(
                                            response != crate::processing::Response::film()
                                                && editable,
                                        ),
                                        tool: None,
                                        default_open: Some(false),
                                    },
                                    |ui| {
                                        ui.add_enabled_ui(editable, |ui| {
                                            let edit = ui::ParameterRow::new(
                                                "toe",
                                                "Toe",
                                                crate::processing::MIN_TOE
                                                    ..=crate::processing::MAX_TOE,
                                            )
                                            .default_value(crate::processing::DEFAULT_TOE)
                                            .show(ui, &mut response.toe)
                                            .edit;
                                            self.roll_control(edit, cx, |project| {
                                                if let InputSettings::Negative {
                                                    response: value,
                                                    ..
                                                } = &mut project.input
                                                {
                                                    value.toe = response.toe;
                                                }
                                            });
                                            let edit = ui::ParameterRow::new(
                                                "shoulder",
                                                "Shoulder",
                                                0.0..=crate::processing::MAX_SHOULDER,
                                            )
                                            .default_value(0.)
                                            .step(0.0001)
                                            .decimals(4)
                                            .help("Highlight roll-off; 0 turns the shoulder off.")
                                            .show(ui, &mut response.shoulder)
                                            .edit;
                                            self.roll_control(edit, cx, |project| {
                                                if let InputSettings::Negative {
                                                    response: value,
                                                    ..
                                                } = &mut project.input
                                                {
                                                    value.shoulder = response.shoulder;
                                                }
                                            });
                                        });
                                    },
                                );
                                if reset.reset {
                                    self.roll_control(
                                        ui::ControlResponse::action(
                                            ui.make_persistent_id("reset-response"),
                                            true,
                                        ),
                                        cx,
                                        |project| {
                                            if let InputSettings::Negative { response, .. } =
                                                &mut project.input
                                            {
                                                *response = crate::processing::Response::film();
                                            }
                                        },
                                    );
                                }
                            }
                        }

                        ui.scope(|ui| {
                            let mut wb = grade.roll.wb;
                            let (responses, pick) = white_balance::controls(
                                ui,
                                "roll-white-balance",
                                &mut wb,
                                ui::ColorControl::RollWb,
                                editable,
                                can_pick,
                            );
                            for response in responses {
                                self.roll_control(response, cx, |project| project.roll.wb = wb);
                            }
                            if pick {
                                start_pick = Some(|cx| Box::pin(white_balance::pick_roll(cx)));
                            }
                            let mut exposure = grade.roll.exposure;
                            let (_, action) = ui::section(
                                ui,
                                "roll-exposure",
                                "Exposure",
                                SectionHeader {
                                    summary: &format!("{:+.2} EV", exposure),
                                    changed: exposure != 0.,
                                    reset: Some(exposure != 0. && editable),
                                    tool: None,
                                    default_open: Some(false),
                                },
                                |ui| {
                                    ui.add_enabled_ui(editable, |ui| {
                                        let response = ui::ParameterRow::new(
                                            "exposure",
                                            "Exposure",
                                            -crate::processing::MAX_EXPOSURE
                                                ..=crate::processing::MAX_EXPOSURE,
                                        )
                                        .default_value(0.)
                                        .slider_range(-3.0..=3.0)
                                        .step(0.05)
                                        .speed(0.01)
                                        .unit("EV")
                                        .slider_help(
                                            "Slider: ±3 EV; type a value for up to ±16 EV. Shift-drag for finer steps.",
                                        )
                                        .show(ui, &mut exposure)
                                        .edit;
                                        self.roll_control(response, cx, |project| {
                                            project.roll.exposure = exposure
                                        });
                                    });
                                },
                            );
                            if action.reset {
                                self.roll_control(
                                    ui::ControlResponse::action(
                                        ui.make_persistent_id("reset-roll-exposure"),
                                        true,
                                    ),
                                    cx,
                                    |project| project.roll.exposure = 0.,
                                );
                            }
                        });
                    });
                } else if self.tab == 1 {
                    if self.adjustment_layer_controls(ui, cx, editable, can_pick) {
                        start_pick = Some(|cx| Box::pin(white_balance::pick_layer(cx)));
                    }
                } else {
                    let show_texture = grade.input.capabilities().grain;
                    if show_texture {
                        let allowed = !self.interaction_blocked();
                        ui.add_enabled_ui(allowed, |ui| {
                            ui::section_with_lead(
                                ui,
                                "film-texture",
                                "Film texture",
                                SectionHeader {
                                    summary: "",
                                    changed: false,
                                    reset: None,
                                    tool: None,
                                    default_open: Some(true),
                                },
                                ui::SectionLead::First,
                                |ui| {
                                    ui.indent("film-controls", |ui| {
                                        self.grain_preset_controls(ui);
                                        self.grain_controls(ui);
                                        self.halation_controls(ui);
                                    });
                                },
                            );
                        });
                    }
                    let lut_lead = if show_texture {
                        ui::SectionLead::Following
                    } else {
                        ui::SectionLead::First
                    };
                    self.look_controls(ui, cx, lut_lead);
                    ui.add_enabled_ui(!self.interaction_blocked(), |ui| {
                        let changed = grade.output.display_target != OutputTarget::DisplayP3;
                        let (_, action) = ui::section(
                            ui,
                            "rendering-target",
                            "Target",
                            SectionHeader {
                                summary: if grade.output.display_target == OutputTarget::DisplayP3
                                {
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
                                ui::inspector_dropdown(
                                    ui,
                                    "rendering-target",
                                    "Target",
                                    if grade.output.display_target == OutputTarget::DisplayP3 {
                                        "Display P3"
                                    } else {
                                        "sRGB"
                                    },
                                    |ui| {
                                        for (target, label) in [
                                            (OutputTarget::DisplayP3, "Display P3"),
                                            (OutputTarget::Srgb, "sRGB"),
                                        ] {
                                            if ui
                                                .selectable_label(
                                                    grade.output.display_target == target,
                                                    label,
                                                )
                                                .clicked()
                                            {
                                                self.output_control(
                                                    ui::ControlResponse::action(
                                                        ui.make_persistent_id("target"),
                                                        true,
                                                    ),
                                                    |output| output.display_target = target,
                                                );
                                            }
                                        }
                                    },
                                )
                                .response
                                .on_hover_text("Output colour space, not the monitor profile.");
                            },
                        );
                        if action.reset {
                            self.output_control(
                                ui::ControlResponse::action(
                                    ui.make_persistent_id("reset-target"),
                                    true,
                                ),
                                |output| output.display_target = OutputTarget::DisplayP3,
                            );
                        }
                    });
                }
            });

        self.preferences.sections = ui::collect_expansion(ui.ctx());
        self.preferences.scopes_visible = self.scopes.visible;
        self.preferences.scope_height_ratio = self.scopes.height_ratio;
        if previous_orientation != (self.document.project.orientation, self.photo_orientation()) {
            self.document.visit.canvas.viewport.fit();
        }
        if let Some(target) = stage_target {
            self.request_input(target, cx);
        }
        if toggle_pivot {
            self.toggle_gamma_pivot();
        }
        if let Some(start) = start_pick
            && let Err(error) = self.start_interaction(cx, start)
        {
            self.status = error;
        }
        self.tab = next_tab;
        scope_slot
    }
}

impl host::App for Editor {
    const INITIAL_WINDOW_SIZE: winit::dpi::LogicalSize<f64> =
        winit::dpi::LogicalSize::new(1100.0, 800.0);

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

    fn processing_revision(&self) -> Option<(u64, u64)> {
        let source = self.document.visit.current_source?;
        self.document
            .visit
            .working_view
            .revision()
            .map(|revision| (source.0, revision))
    }

    fn on_exit(&mut self) {
        self.document.visit.working_view.clear();
        if let Some(developments) = &self.developments {
            developments.close();
        }
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
        let context = ui.ctx().clone();
        self.activities.set_context(&context);
        self.workspace_input.set_context(&context);
        if self.allow_close {
            context.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if !self.started {
            self.started = true;
            ui::configure_branding(ui);
            ui::configure_fonts(&context);
            // Named font families become available at the start of the next pass.
            context.request_discard("Initialize editor typography");
            return;
        }

        // Egui can close a popup or clear focus before later handlers see the same Escape.
        let popup_was_open = egui::Popup::is_any_open(&context);
        let text_focus_id = ui.id().with("editor-text-focus");
        let text_was_focused =
            context.data(|data| data.get_temp::<bool>(text_focus_id).unwrap_or(false));
        let panel_escape_available = !popup_was_open
            && !(self.menus.text_focused && ui.input(|input| input.key_pressed(egui::Key::Escape)));

        self.refresh_canvas_preview();
        self.update_fullscreen(&context);
        let command = self.take_menu_command(ui);
        if self.interaction_blocked() {
            context.request_repaint_after(Duration::from_millis(50));
        }
        self.workspace_shortcuts(ui, cx, popup_was_open, text_was_focused);

        let was_dirty = self.document.dirty();
        self.disable_blocked_workspace(ui);
        let return_requested = self.fullscreen.is_none() && self.workspace_header(ui);
        if return_requested
            || command.is_some_and(|command| !self.menu_targets_text(command.action))
        {
            ui::surrender_keyboard_focus(&context);
        }

        self.workspace_status(ui);
        let strip_demand = self.show_filmstrip(ui, cx);
        if self.dialogs.is_open() {
            ui.disable();
        }
        let scope_slot = self.show_inspector(ui, cx);
        let menu_zoom = command.and_then(|command| viewer_ui::menu_zoom(command.action));
        let mut demand = self.show_viewer(ui, menu_zoom, cx);
        demand.append(strip_demand);
        demand.publish_demand(self, &context, cx);
        let text_focused = context.text_edit_focused();
        context.data_mut(|data| data.insert_temp(text_focus_id, text_focused));
        self.paint_scope(scope_slot);
        if self.document.dirty() != was_dirty {
            context.request_repaint();
        }

        self.application_panel(&context, panel_escape_available);
        // A workspace control may have opened a dialog or started a transaction.
        if command.is_none_or(|command| self.menu_command_available(command)) {
            if return_requested && !command.is_some_and(menu::Command::is_file) {
                self.queue_external_return(cx);
            } else if let Some(command) = command {
                self.run_menu_command(command, &context, cx);
            }
        }
        self.show_document_overlays(&context, cx, !popup_was_open);

        if self.allow_close {
            context.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.menus.text_focused = context.text_edit_focused();
        self.menus.history_input_blocked =
            egui::Popup::is_any_open(&context) || ui.input(|input| input.pointer.any_down());
    }

    fn workspace_shortcuts(
        &mut self,
        ui: &mut egui::Ui,
        cx: &UiContext<'_, Self>,
        popup_was_open: bool,
        text_was_focused: bool,
    ) {
        let workspace_available = !self.workspace_input.is_blocked();
        if self
            .document
            .roll_order
            .handle_drag_shortcuts(ui.ctx(), workspace_available)
        {
            return;
        }

        let fullscreen_shortcut_available = !popup_was_open && self.menus.panel.is_none();
        self.fullscreen_shortcuts(ui, text_was_focused, fullscreen_shortcut_available);
        if !workspace_available
            || (text_was_focused && ui.input(|input| input.key_pressed(egui::Key::Escape)))
            || egui::Popup::is_any_open(ui.ctx())
        {
            return;
        }

        self.fit_view_shortcut(ui.ctx());
        if ui.memory(|memory| {
            memory
                .focused()
                .is_none_or(|id| Some(id) == self.canvas_frame.input_id)
        }) {
            self.select_active_shortcut(ui.ctx());
            self.tool_shortcuts(ui, cx);
            if !self.showing_lightbox() {
                self.roll_navigation_shortcut(ui.ctx(), cx);
            }
        }
    }

    pub(super) fn disable_blocked_workspace(&self, ui: &mut egui::Ui) {
        if self.workspace_input.is_blocked() {
            ui.disable();
        }
    }

    fn workspace_header(&self, ui: &mut egui::Ui) -> bool {
        // The menu bar belongs to the platform; the header keeps the project identity.
        let mut return_requested = false;
        ui::app_header(
            ui,
            |ui| self.project_identity(ui),
            |ui| return_requested = self.external_return_button(ui),
        );
        return_requested
    }

    fn project_identity(&self, ui: &mut egui::Ui) {
        ui.separator();
        let name = self
            .document
            .path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into());
        let label = format!(
            "{name}{}",
            if self.document.dirty() {
                " • Modified"
            } else {
                ""
            }
        );
        let detail = self
            .document
            .path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "Save to create a project document".into());
        ui.add(egui::Label::new(label).truncate())
            .on_hover_text(detail);
    }

    fn workspace_status(&self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status")
            .exact_size(ui::ACTIVITY_BAR_HEIGHT)
            .show(ui, |ui| {
                if let Some(view) = self.activities.view() {
                    view.compact(ui);
                } else if let Some((status, error)) = self.preview_failure_status() {
                    status.compact(ui).on_hover_text(error.to_string());
                } else {
                    ui.add(egui::Label::new(&self.status).truncate())
                        .on_hover_text(&self.status);
                }
            });
    }

    fn show_document_overlays(
        &mut self,
        context: &egui::Context,
        cx: &UiContext<'_, Self>,
        escape_available: bool,
    ) {
        if self.dialogs.is_open() {
            self.dialogs.paint(context, escape_available);
            return;
        }

        self.export_dialogs(context, escape_available, cx);
        if self.dialogs.is_open() {
            // The export form submitted a decision after the dialog check.
            context.request_repaint();
        } else {
            self.activities.modal(
                context,
                egui::Id::new("working-document"),
                escape_available && !self.menus.text_focused,
            );
        }
    }
}
