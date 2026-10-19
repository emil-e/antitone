use super::*;
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

pub(super) struct FlatEdit {
    pub id: u64,
    pub ready: Option<Ready>,
    pub viewport: Viewport,
    pub source: bool,
}

pub(super) struct FlatPreview {
    pub ready: Ready,
    pub region: Option<Rect>,
}

impl Editor {
    pub(super) fn flat_controls(&mut self, ui: &mut egui::Ui) {
        let editing = self.pick == Some(Pick::Flat);
        let settings = self.document.project.flat_field.as_ref();
        let summary = match settings {
            Some(settings) if settings.enabled => "Enabled · check calibration after changes",
            Some(_) => "Disabled",
            None => "Not configured",
        };
        presentation::section(
            ui,
            "roll-flat-field",
            "Flat-field correction",
            presentation::SectionHeader {
                summary,
                changed: settings.is_some(),
                reset: None,
                default_open: if editing { None } else { Some(false) },
            },
            |ui| {
                if editing {
                    controls::active_tool(
                        ui,
                        icons::Icon::Crop,
                        "Flat-field area",
                        None,
                        |ui, _| {
                            if let Some(view) = self.tasks.view() {
                                view.inline(ui);
                            }
                        },
                    );
                }
                ui.add_enabled_ui(
                    !self.busy
                        && self.pick.is_none()
                        && self.crop_edit.is_none()
                        && self.auto_crop_edit.is_none()
                        && self.mask_edit.is_none(),
                    |ui| {
                        if presentation::TextAction::new("Choose reference RAW…")
                            .show(ui)
                            .clicked()
                            && let Some(path) = crate::platform::menu::with_native_dialog(|| {
                                rfd::FileDialog::new()
                                    .set_title("Bare light source · same setup, without film")
                                    .add_filter("Camera RAW files", files::RAW_EXTENSIONS)
                                    .pick_file()
                            })
                        {
                            self.control_command(editing::Command::FlatEdit(path, None));
                        }
                        if let Some(settings) = self.document.project.flat_field.clone() {
                            let path = self.document.source_path(settings.reference).ok();
                            ui.label(
                                path.as_ref()
                                    .and_then(|p| p.file_name())
                                    .map(|name| name.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| "Reference RAW".into()),
                            );
                            let mut enabled = settings.enabled;
                            if ui.checkbox(&mut enabled, "Enabled").changed() {
                                self.control_command(editing::Command::Correction(Some(enabled)));
                            }
                            ui.horizontal(|ui| {
                                if presentation::TextAction::new("Edit area…")
                                    .show(ui)
                                    .clicked()
                                    && let Some(path) = path
                                {
                                    let [x, y, width, height] = settings.map.area();
                                    self.control_command(editing::Command::FlatEdit(
                                        path,
                                        Some(Rect {
                                            x,
                                            y,
                                            width,
                                            height,
                                        }),
                                    ));
                                }
                                if presentation::TextAction::new("Clear").show(ui).clicked() {
                                    self.control_command(editing::Command::Correction(None));
                                }
                            });
                        }
                    },
                );
                ui.small(
                    "Bare light source, same lens/focus/aperture/light. Set this before calibration. Existing Dmin/Dmax stay unchanged; re-pick them after changing correction.",
                );
            },
        );
    }

    pub(super) fn begin_flat_edit(&mut self, path: PathBuf, region: Option<Rect>) {
        if self.busy
            || self.pick.is_some()
            || self.crop_edit.is_some()
            || self.auto_crop_edit.is_some()
            || self.mask_edit.is_some()
        {
            return;
        }
        self.sync_document();
        self.thumbnails.cancel();
        let context = FlatEdit {
            id: self.id,
            ready: self.ready.take(),
            viewport: self.viewport,
            source: self.source,
        };
        self.flat_edit = Some(context);
        self.pick = Some(Pick::Flat);
        self.region = region.map(Coverage::rectangle);
        self.source = true;
        self.viewport.fit();
        self.original.invalidate();
        self.id = self.request_id();
        self.source_epoch
            .store(self.id, std::sync::atomic::Ordering::Relaxed);
        self.busy = true;
        let expected = self
            .document
            .find_source(&path)
            .and_then(|id| self.document.project.source(id).ok())
            .cloned()
            .map(Box::new);
        let wb = self
            .document
            .project
            .development
            .as_ref()
            .map(|d| d.fixed_wb);
        let cache = expected
            .as_deref()
            .map(|source| LoadCache::new(&path, Some(source), wb, &self.document));
        self.status = "Preparing uncorrected flat-field reference…".into();
        self.tasks.begin(Task::Reference(self.id));
        if self
            .commands
            .send(Command::FlatPreview(
                self.id,
                path,
                wb,
                expected,
                self.document.project.camera_basis.clone(),
                cache,
            ))
            .is_err()
        {
            self.busy = false;
            self.tasks.finish(TaskRequest::Reference(self.id));
            self.status = "RAW worker stopped".into();
            self.finish_flat_edit();
            self.report_failure(FailureKind::FlatReference, self.status.clone());
        }
    }

    pub(super) fn finish_flat_edit(&mut self) {
        let Some(context) = self.flat_edit.take() else {
            return;
        };
        self.tasks.finish(TaskRequest::Reference(self.id));
        let _ = self.commands.send(Command::CancelFlat(self.id));
        self.id = context.id;
        self.source_epoch
            .store(self.id, std::sync::atomic::Ordering::Relaxed);
        self.ready = context.ready;
        self.viewport = context.viewport;
        self.source = context.source;
        self.pick = None;
        self.region = None;
        self.pick_drag = None;
        self.last_layout = None;
        self.original.invalidate();
    }

    pub(super) fn apply_flat_edit(&mut self) {
        let Some(coverage) = self.region else {
            return;
        };
        let Some(ready) = self.ready.as_ref() else {
            return;
        };
        let Some(region) = coverage.exact_rectangle(ready.area) else {
            self.status = "Flat-field support must be an integer source rectangle".into();
            self.report_failure(FailureKind::FlatSupport, self.status.clone());
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
        self.tasks.begin(Task::Correction(self.id));
        self.status = "Estimating and validating full-resolution correction…".into();
        if self
            .commands
            .send(Command::FlatAccept(
                self.id,
                region,
                Box::new(self.document.clone()),
                self.current_source,
            ))
            .is_err()
        {
            self.document_pending = None;
            self.history_pending = None;
            self.document_failure = None;
            self.tasks.finish(TaskRequest::Document(self.id));
            self.busy = false;
            self.finish_flat_edit();
            self.status = "RAW worker stopped".into();
            self.report_failure(FailureKind::FlatCorrection, self.status.clone());
        }
    }

    pub(super) fn positive_parameters(
        &self,
        calibration: Calibration,
        roll: RollSettings,
        photo: PhotoSettings,
    ) -> Result<ProcessingParameters, Error> {
        let basis = self
            .document
            .project
            .camera_basis
            .as_ref()
            .ok_or(Error::UnusableSample("no native-camera basis identity"))?;
        let matrix = glam::DMat3::from_cols_array_2d(&basis.camera_to_ap1);
        self.positive_parameters_for_matrix(calibration, roll, photo, matrix)
    }

    pub(super) fn positive_parameters_for_matrix(
        &self,
        calibration: Calibration,
        roll: RollSettings,
        photo: PhotoSettings,
        camera_to_ap1: glam::DMat3,
    ) -> Result<ProcessingParameters, Error> {
        ProcessingParameters::positive(
            self.processing_calibration(calibration)?,
            roll,
            photo,
            camera_to_ap1,
        )
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
