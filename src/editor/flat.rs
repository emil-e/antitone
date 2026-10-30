use super::*;
use crate::processing::InputSettings;
mod operation;
#[cfg(test)]
mod tests;
mod worker;
use crate::{
    gpu::{GpuFlatField, SourceGeometry},
    project::FlatFieldSettings,
};

pub(super) struct Correction {
    pub settings: FlatFieldSettings,
    pub gpu: Arc<GpuFlatField>,
    maximum_gain: f64,
}

impl Correction {
    pub fn prepare(
        device: &wgpu::Device,
        document: &Document,
        previous: Option<&Arc<Self>>,
    ) -> Result<Option<Arc<Self>>, Error> {
        document.validate_for_device(device)?;
        let Some(settings) = document.project.active_flat_field() else {
            return Ok(None);
        };
        if let Some(previous) = previous
            && previous.settings == *settings
        {
            return Ok(Some(previous.clone()));
        }
        let gpu = Arc::new(GpuFlatField::new(device, settings.map.clone())?);
        for source in &document.project.sources {
            if document.project.order.contains(&source.id) {
                settings.require_source(&source.development)?;
                let matrix = glam::DMat3::from_cols_array_2d(&source.development.camera_to_ap1);
                let mut input = if source.development.lens_correction.is_some() {
                    crate::gpu::ProcessingSource::new_continuous(device, &[[0.; 4]], matrix)?
                } else {
                    crate::gpu::ProcessingSource::new(device, &[[0; 4]], matrix)?
                };
                input.set_flat_field(
                    Some(gpu.clone()),
                    SourceGeometry::new(source.development.active_area, [1, 1])?,
                )?;
            }
        }
        let maximum_gain = settings.map.bounds().1.max_element();
        Ok(Some(Arc::new(Self {
            settings: settings.clone(),
            gpu,
            maximum_gain,
        })))
    }
}

pub(super) const DISPLAY: super::picking::Display = super::picking::Display {
    label: "Flat-field",
    view: super::picking::View::Native,
    area: true,
    lock_layer_selection: false,
    help: None,
};

/// Document-owned setup and its temporary reference view.
pub(super) struct FlatEdit {
    pub view: super::canvas_view::CanvasView,
    pub region: Option<Coverage>,
    pub task: Option<async_executor::Task<()>>,
    pub selection: Option<futures_channel::oneshot::Sender<(u64, Rect)>>,
    pub activity: Option<TaskRequest>,
}

pub(super) struct FlatPreview {
    pub ready: Ready,
    pub region: Option<Rect>,
}

impl Editor {
    pub(super) fn flat_controls(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) {
        let editing = self.document.flat_edit().is_some();
        let settings = self.document.state.project.flat_field();
        let configured = settings.is_some();
        let available = self.tool_start_available();
        let summary = match settings {
            Some(settings) if settings.enabled => "Enabled",
            Some(_) => "Disabled",
            None => "Not configured",
        };
        let (_, action) = presentation::section(
            ui,
            "roll-flat-field",
            "Flat-field correction",
            presentation::SectionHeader {
                summary,
                changed: configured,
                reset: Some(configured && !self.busy && !self.tool_interacting()),
                tool: None,
                default_open: if editing { None } else { Some(false) },
            },
            |ui| {
                if editing {
                    ui.small("Select the illuminated area, excluding the holder and including the image centre.");
                    ui.small("Dmin/Dmax stay unchanged; re-pick them after applying correction.");
                    return;
                }
                ui.add_enabled_ui(available, |ui| {
                    if let Some(settings) = self.document.state.project.flat_field() {
                        let path = self.document.source_path(settings.reference).ok();
                        let mut enabled = settings.enabled;
                        if ui.checkbox(&mut enabled, "Enabled").changed() {
                            self.control_command(editing::Command::Correction(Some(enabled)), cx);
                        }
                        let name = path
                            .as_ref()
                            .and_then(|p| p.file_name())
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "Reference RAW".into());
                        let response = ui.add(egui::Label::new(name).truncate());
                        if let Some(path) = path {
                            response.on_hover_text(path.display().to_string());
                        }
                    } else if presentation::TextAction::new("Set up…")
                        .help("Choose a film-free RAW of the light source from the same scanning setup.")
                        .show(ui)
                        .clicked()
                        && let Some(path) = crate::platform::menu::with_native_dialog(|| {
                            rfd::FileDialog::new()
                                .set_title("Bare light · same lens, focus, aperture and light · without film")
                                .add_filter("Camera RAW files", files::RAW_EXTENSIONS)
                                .pick_file()
                        })
                    {
                        self.control_command(editing::Command::FlatEdit(path, None), cx);
                    }
                });
            },
        );
        if action.reset {
            self.control_command(editing::Command::Correction(None), cx);
        }
    }

    pub(super) fn begin_flat_edit(
        &mut self,
        path: PathBuf,
        region: Option<Rect>,
        cx: &UiContext<'_, Self>,
    ) {
        if !self.prepare_tool() {
            return;
        }
        self.sync_document();
        self.document.thumbnails.cancel();
        let id = self.request_id();
        let (selection, receive) = futures_channel::oneshot::channel();
        let context = FlatEdit {
            view: Default::default(),
            region: region.map(Coverage::rectangle),
            task: None,
            selection: Some(selection),
            activity: Some(TaskRequest::Reference(id)),
        };
        self.document.set_flat_edit(Some(context));
        self.busy = true;
        self.status = "Preparing uncorrected flat-field reference…".into();
        self.tasks.begin(Task::Reference(id));
        let request = self.reference_preview_input(path);
        if let Some(task) = operation::setup(request, self.source_services.clone(), receive, cx) {
            self.document.flat_edit_mut().unwrap().task = Some(task);
        } else {
            self.finish_flat_edit();
            self.status = "Application stopped".into();
            self.report_failure(FailureKind::FlatReference, self.status.clone());
        }
    }

    pub(super) fn reference_preview_input(&self, path: PathBuf) -> source::Request {
        let expected = self
            .document
            .find_source(&path)
            .and_then(|id| self.document.state.project.source(id).ok())
            .cloned();
        // A retained record is either an ordered photo or the current flat-field reference;
        // both belong to the current stage. Use its saved decode identity when present.
        let wb = self
            .document
            .state
            .project
            .acquisition_wb(expected.as_ref());
        let file = self.document.file_key(&path);
        source::Request {
            reference: expected.as_ref().map_or_else(Arc::default, |source| {
                self.document
                    .exposure_slot(&self.document.exposures, source.id)
            }),
            input: self.input(),
            path,
            wb,
            expected,
            file,
            sources: self.source_provider(),
            correction: None,
        }
    }

    pub(super) fn finish_flat_edit(&mut self) {
        let Some(context) = self.document.take_flat_edit() else {
            return;
        };
        if let Some(activity) = context.activity {
            if self.tasks.finish(activity) {
                self.busy = false;
            }
            if let TaskRequest::Document(id) = activity
                && self.document_pending == Some(id)
            {
                self.document_pending = None;
                self.document_failure = None;
                let history = self.history_pending.take();
                self.history_failed(history);
            }
        }
    }

    pub(super) fn apply_flat_edit(&mut self, _cx: &UiContext<'_, Self>) {
        if self.busy || self.document_pending.is_some() {
            return;
        }
        self.sync_document();
        let Some(coverage) = self.pick_region() else {
            return;
        };
        let Some(reference) = self
            .document
            .flat_edit()
            .and_then(|edit| edit.view.ready.as_ref())
        else {
            return;
        };
        let Some(region) = coverage.exact_rectangle(reference.area) else {
            self.status = "Flat-field support must be an integer source rectangle".into();
            self.report_failure(FailureKind::FlatSupport, self.status.clone());
            return;
        };
        let Some(selection) = self
            .document
            .flat_edit_mut()
            .and_then(|edit| edit.selection.take())
        else {
            return;
        };
        self.history_pending = Some(history::Pending::Edit {
            before: Box::new(self.history_snapshot()),
            label: "Correction",
            photo: None,
        });
        self.busy = true;
        self.document_pending = Some(self.id);
        self.document_failure = Some(FailureKind::FlatCorrection);
        self.document.flat_edit_mut().unwrap().activity = Some(TaskRequest::Document(self.id));
        self.tasks.begin(Task::Correction(self.id));
        self.status = "Estimating and validating full-resolution correction…".into();
        if selection.send((self.id, region)).is_err() {
            self.document_prepared(self.id, Err("Application stopped".into()));
        }
    }

    pub(super) fn positive_parameters(
        &self,
        input: impl Into<InputSettings>,
        roll: RollSettings,
    ) -> Result<ProcessingParameters, Error> {
        let basis = self
            .document
            .project
            .camera_basis()
            .ok_or(Error::UnusableSample("no native-camera basis identity"))?;
        let matrix = glam::DMat3::from_cols_array_2d(&basis.camera_to_ap1);
        self.positive_parameters_for_matrix(input, roll, matrix)
    }

    pub(super) fn positive_parameters_for_matrix(
        &self,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        camera_to_ap1: glam::DMat3,
    ) -> Result<ProcessingParameters, Error> {
        let input = input.into();
        let input = match input.calibration() {
            Some(base) => input.with_calibration(self.processing_calibration(base)?),
            None => input,
        };
        ProcessingParameters::interpret(input, roll, camera_to_ap1)
    }

    pub(super) fn processing_calibration(
        &self,
        calibration: Calibration,
    ) -> Result<Calibration, Error> {
        let source_floor = self
            .document
            .visit
            .current_source
            .map(|id| {
                self.document
                    .state
                    .project
                    .source(id)
                    .map(|s| s.development.input_floor)
            })
            .transpose()?
            .unwrap_or(crate::processing::CAMERA_INPUT_FLOOR);
        let residual_gain = self
            .document
            .correction
            .as_ref()
            .map_or(1., |c| c.maximum_gain);
        calibration.for_source(source_floor, residual_gain)
    }
}
