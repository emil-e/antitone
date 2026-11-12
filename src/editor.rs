use crate::Error;
use crate::adjustment_layers::AdjustmentLayerSettings;
use crate::adjustment_layers::AdjustmentLayerSnapshot;
use crate::application::UiContext;
use crate::color::Balance;
use crate::coverage::Coverage;
use crate::crop::Crop;
use crate::export::ExrPrecision;
use crate::geometry::Composition;
use crate::geometry::PhotoOrientation;
use crate::geometry::RollOrientation;
use crate::grain::GrainPreset;
use crate::grain::GrainSettings;
use crate::platform::host;
use crate::processing::Calibration;
use crate::processing::InputSettings;
use crate::processing::RollSettings;
use crate::raw::Rect;
use crate::refinement::ViewRequest;
use crate::rendering::OutputTarget;
use crate::scan::Scan;
use crate::viewer::GpuImage;
use crate::viewer::ImagePaint;
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
#[cfg(test)]
use activity::Reporter as ActivityReporter;
use activity::{Activities, Activity, Options as ActivityOptions};
mod fullscreen;
use fullscreen::FullscreenView;
mod thumbnails;
use thumbnails::Thumbnails;
mod adjustment_layers;
mod look;
mod processing;
use adjustment_layers::AdjustmentLayerEditor;
mod white_balance;
mod working_view;
use working_view::WorkingView;
mod original_view;
use original_view::OriginalPurpose;
mod selected_preview;
use selected_preview::{SelectionPaint, SelectionTiming};
mod scopes;
use scopes::{Scopes, Slot as ScopeSlot};
mod analysis;
#[cfg(test)]
use analysis::AnalyzeTool;
mod dust;
mod flat;
mod spatial;
use flat::Correction;
#[cfg(test)]
use flat::FlatEdit;
mod auto_crop;
mod crop_editor;
use crop_editor::CropEdit;
mod mask_editor;
use mask_editor::MaskEdit;
#[cfg(test)]
mod cache_preferences_tests;
mod calibration;
mod canvas;
mod canvas_view;
mod export;
mod frame;
mod gamma_pivot;
#[cfg(all(test, feature = "ui-tests"))]
mod kittest_tests;
mod measurement;
mod memory;
mod navigation;
mod photo_scope;
mod photo_ui;
mod picking;
mod save;
#[cfg(test)]
use crate::export::Overwrite;
#[cfg(test)]
mod input_tests;
mod roll_selection;
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
                    basis: Arc::new(std::sync::OnceLock::from(crate::project::ScanBasis::new(
                        source,
                    ))),
                    expected_camera: Some(source.development.camera_model.clone()),
                },
            )
        }
        None => crate::source::Input::reference(path, file, wb, input, Arc::default()),
    };
    let developed = source::Request {
        source: physical,
        correction: correction.cloned(),
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
    fullscreen: Option<FullscreenView>,
    canvas_frame: canvas::Frame,
    // Running physical work outlives logical cancellation.
    id: u64,
    look: look::Session,
    status: String,
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
    document: DocumentScope,

    next_request: u64,
    export_services: Option<export::Services>,
    files: files::State,
    allow_close: bool,
    developments: Option<source_loading::SourceLoader>,
    source_services: Option<source::Services>,
    source_epoch: Arc<std::sync::atomic::AtomicU64>,
}

impl Editor {
    fn interaction_blocked(&self) -> bool {
        self.document.look.is_some() || self.activities.blocks_workspace()
    }

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

        editor.submissions = submissions.clone();
        editor.initialize_viewer(&device, &queue);
        editor.look.services = Some(look::Services::new(device.limits()));
        editor.source_services = Some(source::Services {
            developments: developments.clone(),
            renderers: renderers.clone(),
            device: device.clone(),
            queue: queue.clone(),
        });
        editor.export_services = Some(export::Services::new(
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
        Self {
            submissions: crate::platform::SubmissionGate::default(),
            developments: None,
            source_services: None,
            source_epoch: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            scopes: Scopes::default(),
            fullscreen: None,
            canvas_frame: Default::default(),
            started: false,
            menus: menu::Menus::default(),
            id: 0,
            look: Default::default(),
            status: "Add scans to begin".into(),
            activities: Activities::default(),
            #[cfg(test)]
            fixture_activity: None,
            dialogs: Default::default(),

            auto_crop_margin: crate::auto_crop::DEFAULT_MARGIN_PERCENT,

            source_exposure: 0.,
            source_white_balance: Default::default(),
            tab: 0,
            preferences: preferences::Preferences::default(),
            preference_store: None,

            document,
            next_request: 0,
            export_services: None,
            files: Default::default(),
            allow_close: false,
        }
    }

    pub fn restore_ui_preferences(&mut self) {
        match preferences::Store::open_default() {
            Ok((store, preferences)) => {
                self.apply_preferences(preferences);
                self.preference_store = Some(store);
                self.persist_preferences();
                if !self.files.opening() {
                    self.preferences
                        .sections
                        .insert("roll-calibration".into(), true);
                }
            }
            Err(error) => log::warn!(error = error.to_string().as_str(); "preferences_load_failed"),
        }
    }

    fn validate_roll_values(&self, project: &crate::project::Project) -> Result<(), Error> {
        crate::processing::exposure_multiplier(project.roll.exposure)?;
        crate::color::wb_matrix(project.roll.wb)?;
        if let Some(input) = project.input.negative_input() {
            input.parameters()?;
        }
        if project.input.ready() && !project.sources.is_empty() {
            self.validate_processing(project.input, project.roll, self.photo_settings())?;
        }
        Ok(())
    }

    fn roll_control(
        &mut self,
        response: ui::ControlResponse,
        cx: &UiContext<'_, Self>,
        write: impl FnOnce(&mut crate::project::Project),
    ) {
        let Some(mut edit) = self.begin_control(response, false) else {
            return;
        };
        if response.changed {
            let starting = edit.values.initial();
            let previous_request = edit
                .gamma_request
                .unwrap_or_else(|| edit.values.preview().snapshot().unwrap().input);
            let proposed = edit.values.update(|project| {
                let previous = project.input;
                write(project);
                let proposed = project.input;
                project.input = previous;
                proposed
            });
            if proposed != previous_request {
                edit.calculation = None;
                if self.gamma_compensation_required(starting.input, proposed) {
                    edit.gamma_request = Some(proposed);
                    edit.values.rename("Gamma & balance");
                } else {
                    edit.gamma_request = None;
                    edit.values.update(|project| {
                        project.input = proposed;
                        if proposed == starting.input {
                            project.roll.wb = starting.roll.wb;
                        }
                    });
                }
            }
            let validation = if let Some(input) = edit.gamma_request {
                input
                    .negative_input()
                    .map(|input| input.parameters())
                    .transpose()
                    .map(|_| ())
            } else {
                self.validate_roll_values(&edit.values.preview().snapshot().unwrap())
            };
            if let Err(error) = validation {
                self.status = format!("Edit rejected: {error}");
                return;
            }
        }
        let calculate = edit.gamma_request.is_some() && edit.calculation.is_none();
        // The calculation is feature-owned; it retains the edit until its result is ready.
        if calculate {
            edit.finished = !response.active;
            self.document.controls_edit = Some(edit);
            self.compensate_control_edit(cx);
        } else {
            self.retain_control(edit, response, false);
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
        if response.changed {
            let result = edit.values.update(|project| {
                write(&mut project.output);
                project.output.grain.validate()?;
                project.output.halation.validate()
            });
            if let Err(error) = result {
                self.status = format!("Edit rejected: {error}");
                return;
            }
        }
        self.retain_control(edit, response, false);
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
                    output.halation = crate::halation::HalationSettings {
                        enabled: output.halation.enabled,
                        ..preset.halation()
                    };
                },
            );
        }
    }

    fn halation_controls(&mut self, ui: &mut egui::Ui) {
        let mut settings = self.document.project_snapshot().output.halation;
        let changed = settings != crate::halation::HalationSettings::default();
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
                    0.0..=crate::halation::MAX_STRENGTH,
                )
                .default_value(crate::halation::DEFAULT_STRENGTH)
                .help("Redistribute positive light into a warm halo; 0 leaves the image unchanged.")
                .show(ui, &mut settings.strength)
                .edit;
                self.output_control(response, |output| {
                    output.halation.strength = settings.strength
                });
                let response = ui::ParameterRow::new(
                    "halation-size",
                    "Core size",
                    crate::halation::MIN_SIZE..=crate::halation::MAX_SIZE,
                )
                .default_value(crate::halation::DEFAULT_SIZE)
                .slider_range(crate::halation::MIN_SIZE..=0.21)
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
                    crate::grain::MIN_AMOUNT..=crate::grain::MAX_AMOUNT,
                )
                .default_value(crate::grain::DEFAULT_AMOUNT)
                .step(0.001)
                .decimals(3)
                .help("Grain contrast before pixel averaging; 0 removes grain.")
                .show(ui, &mut settings.amount)
                .edit;
                self.output_control(response, |output| output.grain.amount = settings.amount);
                let response = ui::ParameterRow::new(
                    "grain-size",
                    "Size",
                    crate::grain::MIN_SIZE..=crate::grain::MAX_SIZE,
                )
                .default_value(crate::grain::DEFAULT_SIZE)
                .step(0.001)
                .decimals(3)
                .unit("%")
                .help("Scale grain and image softness together in fixed crop-relative units.")
                .show(ui, &mut settings.size)
                .edit;
                self.output_control(response, |output| {
                    output.grain = output.grain.resized(settings.size)
                });
                let response = ui::ParameterRow::new(
                    "grain-colour",
                    "Colour",
                    crate::grain::MIN_COLOUR..=crate::grain::MAX_COLOUR,
                )
                .default_value(crate::grain::DEFAULT_COLOUR)
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
                ).warning().show(context, id, escape)
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
                project.scan = None;
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
        self.document.correction_restore = None;
        self.document.correction = Default::default();
        self.cancel_spatial();
        self.document.hosted_tool = Default::default();
        self.document.thumbnails.cancel();
        self.id = self.request_id();
        self.source_epoch
            .store(self.id, std::sync::atomic::Ordering::Relaxed);
        self.reinterpret_photo(self.id);
        self.invalidate_original();
        if let Some(id) = self.document.visit.current_source
            && let Ok(path) = self.document.source_path(id)
        {
            self.start_source_acquisition(path, cx);
        }
    }

    fn request_id(&mut self) -> u64 {
        self.next_request += 1;
        self.next_request
    }

    fn controls(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) -> Option<ScopeSlot> {
        use ui::ColorControl;
        use ui::SectionHeader;

        let grade = self.document.project_snapshot();
        let requested_input = self
            .document
            .controls_edit
            .as_ref()
            .and_then(|edit| edit.gamma_request);
        let mut stage_target = None;
        let mut toggle_pivot = false;
        let previous_orientation = (self.document.project.orientation, self.photo_orientation());
        ui::inspector_spacing(ui);
        // The inspector column width is authoritative; content must not widen the panel.
        let inspector_width = ui.available_width();
        ui.set_width(inspector_width);
        ui::restore_expansion(ui.ctx(), self.preferences.sections.clone());
        let scope_slot = self.scope_panel(ui);
        ui.add_space(8.);

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
            self.roll_tool_shelf(ui, cx);
        } else if self.tab == 1 {
            self.photo_tool_shelf(ui, cx);
        }

        let gamma_pending = self.controls_pending();
        let editable = !self.interaction_blocked()
            && !self.tool_interacting()
            && !gamma_pending
            && grade.input.ready();
        let can_pick = self.tool_start_available() && self.selected_area().is_some();
        let mut start_pick: Option<editing::Interaction> = None;

        egui::ScrollArea::vertical()
            .id_salt(("inspector", self.tab))
            .show(ui, |ui| {
                if self.document.hosted_tool.occupied() {
                    self.hosted_drawer(ui, cx);
                } else if self.tab == 1 {
                    ui.add_enabled_ui(!self.interaction_blocked(), |ui| self.hosted_drawer(ui, cx));
                }
                if self.tab == 0 {
                    let displayed_input = grade.input;
                    let input = displayed_input;
                    ui.push_id("roll", |ui| {
                        let capabilities = input.capabilities();
                        ui.horizontal(|ui| {
                            ui.label("Input stage");
                            ui.add_enabled_ui(!self.tool_interacting(), |ui| {
                                egui::ComboBox::from_id_salt("input-stage")
                                    .selected_text(if input.is_negative() {
                                        "Negative"
                                    } else {
                                        "Digital"
                                    })
                                    .show_ui(ui, |ui| {
                                        if input.is_negative() {
                                            let _ = ui.selectable_label(true, "Negative");
                                        } else if ui.selectable_label(false, "Negative").clicked() {
                                            stage_target = Some(self.negative_input());
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
                                    )
                            });
                        });
                        if capabilities.flat_field {
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
                        ui.scope(|ui| {
                            if let InputSettings::Negative {
                                mut response,
                                mut gamma,
                                mut balance,
                                ..
                            } = requested_input.unwrap_or(grade.input)
                            {
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
                                        tool: Some(ui::balance_pick_tool(
                                            can_pick,
                                            "Pick neutral for gamma balance",
                                        )),
                                        default_open: Some(true),
                                    },
                                    self.gamma_pivot_tool(can_pick),
                                    |ui| {
                                        ui.add_enabled_ui(
                                            editable
                                                || gamma_pending
                                                    && !self.interaction_blocked()
                                                    && !self.tool_interacting(),
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
                                            editable
                                                || gamma_pending
                                                    && !self.interaction_blocked()
                                                    && !self.tool_interacting(),
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
                                if reset.secondary_tool {
                                    toggle_pivot = true;
                                }
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
                            let mut wb = grade.roll.wb;
                            let (responses, pick) = white_balance::controls(
                                ui,
                                "roll-white-balance",
                                &mut wb,
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
                    if grade.input.capabilities().grain {
                        let allowed = !self.interaction_blocked()
                            && !self.tool_interacting()
                            && !self.controls_pending();
                        ui.add_enabled_ui(allowed, |ui| {
                            ui::section(
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
                    self.look_controls(ui, cx);
                    ui.add_enabled_ui(!self.tool_interacting() && !self.controls_pending(), |ui| {
                        let changed = grade.output.tiff != crate::project::TiffTarget::DisplayP3;
                        let (_, action) = ui::section(
                            ui,
                            "rendering-target",
                            "Target",
                            SectionHeader {
                                summary: if grade.output.tiff
                                    == crate::project::TiffTarget::DisplayP3
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
                                    if grade.output.tiff == crate::project::TiffTarget::DisplayP3 {
                                        "Display P3"
                                    } else {
                                        "sRGB"
                                    },
                                    |ui| {
                                        for (target, label) in [
                                            (crate::project::TiffTarget::DisplayP3, "Display P3"),
                                            (crate::project::TiffTarget::Srgb, "sRGB"),
                                        ] {
                                            if ui
                                                .selectable_label(
                                                    grade.output.tiff == target,
                                                    label,
                                                )
                                                .clicked()
                                            {
                                                self.output_control(
                                                    ui::ControlResponse::action(
                                                        ui.make_persistent_id("target"),
                                                        true,
                                                    ),
                                                    |output| output.tiff = target,
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
                                |output| output.tiff = crate::project::TiffTarget::DisplayP3,
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
        if self.tab != 1 {
            self.document.visit.controls_edit = None;
        }
        if let Some(target) = stage_target {
            self.cancel_control_edit();
            self.request_input(target, cx);
        }
        if toggle_pivot {
            self.toggle_gamma_pivot(cx);
        }
        if let Some(start) = start_pick {
            self.cancel_control_edit();
            if let Err(error) = self.start_interaction(cx, start) {
                self.status = error;
            }
        }
        self.tab = next_tab;
        scope_slot
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
        self.document
            .visit
            .working_view
            .revision()
            .map(|revision| (self.id, revision))
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
        self.activities.set_context(ui.ctx());
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

        let was_dirty = self.document.dirty();
        self.disable_blocked_workspace(ui);
        self.workspace_chrome(ui, cx, &mut actions);
        let scope_slot = self.edit_frame_controls(ui, cx);

        let menu_zoom = match actions.menu {
            Some(crate::platform::menu::Action::ActualSize) => Some(1.),
            Some(crate::platform::menu::Action::ZoomIn) => Some(-1.25),
            Some(crate::platform::menu::Action::ZoomOut) => Some(-1. / 1.25),
            _ => None,
        };
        self.show_viewer(ui, menu_zoom, cx);
        let text_focused = ui.ctx().text_edit_focused();
        ui.ctx()
            .data_mut(|data| data.insert_temp(ui.id().with("editor-text-focus"), text_focused));
        self.paint_scope(scope_slot);
        if self.document.dirty() != was_dirty {
            ui.ctx().request_repaint();
        }

        // Platform menu and file intents are revalidated after workspace input.
        self.application_panel(ui.ctx(), panel_escape_available);
        self.dispatch_frame_actions(ui, cx, actions);
        self.frame_dialogs(ui, cx, !popup_was_open);
        if self.allow_close {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.menus.text_focused = ui.ctx().text_edit_focused();
        self.menus.history_input_blocked =
            egui::Popup::is_any_open(ui.ctx()) || ui.input(|input| input.pointer.any_down());
    }
}
