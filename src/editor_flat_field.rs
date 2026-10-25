use super::*;
use crate::processing::InputSettings;
#[cfg(test)]
#[path = "editor_flat_field_tests.rs"]
mod tests;
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

/// Reference session and saved working context, independent of temporary visits.
pub(super) struct FlatEdit {
    pub working: Option<Ready>,
    pub reference: Option<FlatPreview>,
    pub viewport: Viewport,
    pub source: bool,
    pub task: Option<async_executor::Task<()>>,
}

pub(super) struct FlatPreview {
    pub ready: Ready,
    pub region: Option<Rect>,
}

impl Editor {
    pub(super) fn flat_controls(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) {
        let editing = self.pick == Some(Pick::Flat);
        let settings = self.document.project.flat_field();
        let configured = settings.is_some();
        let available = !self.busy
            && self.pick.is_none()
            && self.document.visit.crop_edit.is_none()
            && self.document.auto_crop_edit.is_none()
            && self.document.visit.mask_edit.is_none();
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
                reset: Some(configured && available),
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
                    if let Some(settings) = self.document.project.flat_field() {
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
        if self.busy
            || self.pick.is_some()
            || self.document.visit.crop_edit.is_some()
            || self.document.auto_crop_edit.is_some()
            || self.document.visit.mask_edit.is_some()
        {
            return;
        }
        self.sync_document();
        self.thumbnails.cancel();
        let context = FlatEdit {
            working: self.document.visit.ready.take(),
            reference: None,
            viewport: self.viewport,
            source: self.source,
            task: None,
        };
        self.flat_edit = Some(context);
        self.pick = Some(Pick::Flat);
        self.region = region.map(Coverage::rectangle);
        self.source = true;
        self.viewport.fit();
        self.invalidate_original();
        self.id = self.request_id();
        self.replace_photo_scope(self.id);
        self.source_epoch
            .store(self.id, std::sync::atomic::Ordering::Relaxed);
        self.busy = true;
        self.status = "Preparing uncorrected flat-field reference…".into();
        self.tasks.begin(Task::Reference(self.id));
        let request = self.reference_preview_input(path);
        self.preview_reference(request, cx);
    }

    pub(super) fn reference_preview_input(&self, path: PathBuf) -> source::Request {
        let expected = self
            .document
            .find_source(&path)
            .and_then(|id| self.document.project.source(id).ok())
            .cloned();
        // A retained record is either an ordered photo or the current flat-field reference;
        // both belong to the current stage. Use its saved decode identity when present.
        let wb = self.document.project.acquisition_wb(expected.as_ref());
        let cache = expected
            .as_ref()
            .map(|source| LoadCache::new(&path, Some(source), &self.document));
        source::Request {
            path,
            wb,
            expected,
            cache,
            correction: None,
        }
    }

    pub(super) fn reference_input(
        &self,
    ) -> Result<Option<super::reference_services::Acceptance>, &'static str> {
        let Some(coverage) = self.region else {
            return Ok(None);
        };
        let Some(reference) = self
            .flat_edit
            .as_ref()
            .and_then(|edit| edit.reference.as_ref())
        else {
            return Ok(None);
        };
        let ready = &reference.ready;
        let region = coverage
            .exact_rectangle(ready.area)
            .ok_or("Flat-field support must be an integer source rectangle")?;
        Ok(Some(super::reference_services::Acceptance {
            region,
            document: self.document.capture(),
            selected: self.current_source,
            source: source::Context {
                source: ready.developed.clone(),
                correction: None,
            },
            record: ready.record.clone(),
            correction: self.correction.clone(),
        }))
    }

    pub(super) fn finish_flat_edit(&mut self) {
        let Some(context) = self.flat_edit.take() else {
            return;
        };
        if self.tasks.finish(TaskRequest::Reference(self.id)) {
            self.busy = false;
        }
        if self.document_pending == Some(self.id) {
            self.document_pending = None;
            self.document_failure = None;
            let history = self.history_pending.take();
            self.history_failed(history);
            if self.tasks.finish(TaskRequest::Document(self.id)) {
                self.busy = false;
            }
        }
        self.id = self.request_id();
        self.replace_photo_scope(self.id);
        self.source_epoch
            .store(self.id, std::sync::atomic::Ordering::Relaxed);
        self.document.visit.ready = context.working;
        self.viewport = context.viewport;
        self.source = context.source;
        self.pick = None;
        self.region = None;
        self.pick_drag = None;
        self.last_layout = None;
        self.invalidate_original();
    }

    pub(super) fn apply_flat_edit(&mut self, cx: &UiContext<'_, Self>) {
        if self.busy || self.document_pending.is_some() {
            return;
        }
        self.sync_document();
        let input = match self.reference_input() {
            Ok(Some(input)) => input,
            Ok(None) => return,
            Err(message) => {
                self.status = message.into();
                self.report_failure(FailureKind::FlatSupport, self.status.clone());
                return;
            }
        };
        self.history_pending = Some(history::Pending::Edit {
            before: Box::new(self.history_snapshot()),
            label: "Correction",
            photo: None,
        });
        self.busy = true;
        self.document_pending = Some(self.id);
        self.document_failure = Some(FailureKind::FlatCorrection);
        self.tasks.begin(Task::Correction(self.id));
        self.status = "Estimating and validating full-resolution correction…".into();
        self.accept_reference(input, cx);
    }

    pub(super) fn positive_parameters(
        &self,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        photo: PhotoSettings,
    ) -> Result<ProcessingParameters, Error> {
        let basis = self
            .document
            .project
            .camera_basis()
            .ok_or(Error::UnusableSample("no native-camera basis identity"))?;
        let matrix = glam::DMat3::from_cols_array_2d(&basis.camera_to_ap1);
        self.positive_parameters_for_matrix(input, roll, photo, matrix)
    }

    pub(super) fn positive_parameters_for_matrix(
        &self,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        photo: PhotoSettings,
        camera_to_ap1: glam::DMat3,
    ) -> Result<ProcessingParameters, Error> {
        let input = input.into();
        let input = match input.calibration() {
            Some(base) => input.with_calibration(self.processing_calibration(base)?),
            None => input,
        };
        ProcessingParameters::interpret(input, roll, photo, camera_to_ap1)
    }

    pub(super) fn processing_calibration(
        &self,
        calibration: Calibration,
    ) -> Result<Calibration, Error> {
        let source_floor = self
            .current_source
            .map(|id| {
                self.document
                    .project
                    .source(id)
                    .map(|s| s.development.input_floor)
            })
            .transpose()?
            .unwrap_or(crate::processing::CAMERA_INPUT_FLOOR);
        let residual_gain = self.correction.as_ref().map_or(1., |c| c.maximum_gain);
        calibration.for_source(source_floor, residual_gain)
    }
}
