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
    corrected_floor: f64,
}
impl Correction {
    pub fn prepare(
        device: &wgpu::Device,
        document: &Document,
        previous: Option<&Arc<Self>>,
    ) -> Result<Option<Arc<Self>>, Error> {
        document.validate()?;
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
                let mut input = crate::gpu::ProcessingSource::new(
                    device,
                    &[[0; 4]],
                    glam::DMat3::from_cols_array_2d(&source.development.camera_to_ap1),
                )?;
                input.set_flat_field(
                    Some(gpu.clone()),
                    SourceGeometry::new(source.development.active_area, [1, 1])?,
                )?;
            }
        }
        let corrected_floor = settings.map.precision_floor()?;
        Ok(Some(Arc::new(Self {
            settings: settings.clone(),
            gpu,
            corrected_floor,
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
                            if self.busy {
                                ui.small(if self.ready.is_none() {
                                    "Preparing reference…"
                                } else {
                                    "Applying correction…"
                                });
                            }
                        },
                    );
                }
                ui.add_enabled_ui(
                    !self.busy && self.pick.is_none() && self.crop_edit.is_none(),
                    |ui| {
                        if presentation::TextAction::new("Choose reference RAW…")
                            .show(ui)
                            .clicked()
                            && let Some(path) = rfd::FileDialog::new()
                                .set_title("Bare light source · same setup, without film")
                                .add_filter("Camera RAW", &["arw"])
                                .pick_file()
                        {
                            self.begin_flat_edit(path, None);
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
                                self.sync_document();
                                let mut candidate = self.document.clone();
                                candidate.project.flat_field.as_mut().unwrap().enabled = enabled;
                                self.start_document(DocumentWork::Correction(
                                    candidate,
                                    self.current_source,
                                ));
                            }
                            ui.horizontal(|ui| {
                                if presentation::TextAction::new("Edit area…")
                                    .show(ui)
                                    .clicked()
                                    && let Some(path) = path
                                {
                                    let [x, y, width, height] = settings.map.area();
                                    self.begin_flat_edit(
                                        path,
                                        Some(Rect {
                                            x,
                                            y,
                                            width,
                                            height,
                                        }),
                                    );
                                }
                                if presentation::TextAction::new("Clear").show(ui).clicked() {
                                    self.sync_document();
                                    let mut candidate = self.document.clone();
                                    candidate.project.flat_field = None;
                                    self.start_document(DocumentWork::Correction(
                                        candidate,
                                        self.current_source,
                                    ));
                                }
                            });
                        }
                    },
                );
                ui.small("Bare light source, same lens/focus/aperture/light. Set this before calibration. Existing Dmin/Dmax stay unchanged; re-pick them after changing correction.");
            },
        );
    }
    pub(super) fn begin_flat_edit(&mut self, path: PathBuf, region: Option<Rect>) {
        if self.busy || self.pick.is_some() || self.crop_edit.is_some() {
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
        self.signature = None;
        self.settled = None;
        self.refining = None;
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.id = self.request_id();
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
        self.status = "Preparing uncorrected flat-field reference…".into();
        if self
            .commands
            .send(Command::FlatPreview(
                self.id,
                path,
                wb,
                expected,
                self.document.project.camera_basis.clone(),
            ))
            .is_err()
        {
            self.busy = false;
            self.status = "RAW worker stopped".into();
            self.finish_flat_edit();
        }
    }
    pub(super) fn finish_flat_edit(&mut self) {
        let Some(context) = self.flat_edit.take() else {
            return;
        };
        let _ = self.commands.send(Command::CancelFlat(self.id));
        self.id = context.id;
        self.ready = context.ready;
        self.viewport = context.viewport;
        self.source = context.source;
        self.pick = None;
        self.region = None;
        self.pick_drag = None;
        self.last_layout = None;
        self.signature = None;
        self.settled = None;
        self.refining = None;
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
            return;
        };
        self.sync_document();
        self.busy = true;
        self.document_pending = Some(self.id);
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
            self.busy = false;
            self.finish_flat_edit();
            self.status = "RAW worker stopped".into();
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
        if calibration.input_floor != crate::processing::CAMERA_INPUT_FLOOR {
            return Err(Error::InvalidParameter(
                "editor calibration requires the native-code base floor",
            ));
        }
        let calibration = match &self.correction {
            Some(c) => Calibration {
                input_floor: c.corrected_floor,
                ..calibration
            }
            .validate()?,
            None => calibration,
        };
        ProcessingParameters::positive(calibration, roll, photo, camera_to_ap1)
    }
}
