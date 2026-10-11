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
    floor_ratio: f64,
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
                    glam::DMat3::from_cols_array_2d(&source.development.camera_to_ap0),
                )?;
                input.set_flat_field(
                    Some(gpu.clone()),
                    SourceGeometry::new(source.development.active_area, [1, 1])?,
                )?;
            }
        }
        let matrix = glam::DMat3::from_cols_array_2d(&settings.development.camera_to_ap0);
        let base = (matrix.x_axis.abs() + matrix.y_axis.abs() + matrix.z_axis.abs()).max_element()
            / 65535.;
        let floor_ratio = settings.map.precision_floor(matrix)? / base;
        Ok(Some(Arc::new(Self {
            settings: settings.clone(),
            gpu,
            floor_ratio,
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
        controls::section(ui, "flat-field", "Flat-field correction", None, |ui| {
            ui.add_enabled_ui(!self.busy && self.pick.is_none(), |ui| {
                if ui.button("Choose reference RAW…").clicked()
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
                        if ui.button("Edit area…").clicked()
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
                        if ui.button("Clear").clicked() {
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
            });
            ui.small("Bare light source, same lens/focus/aperture/light. Set this before calibration. Existing Dmin/Dmax stay unchanged; re-pick them after changing correction.");
        });
    }
    pub(super) fn begin_flat_edit(&mut self, path: PathBuf, region: Option<Rect>) {
        if self.busy || self.pick.is_some() {
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
        self.region = region;
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
            .send(Command::FlatPreview(self.id, path, wb, expected))
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
        self.signature = None;
        self.settled = None;
        self.refining = None;
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    pub(super) fn apply_flat_edit(&mut self) {
        let Some(region) = self.region else {
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
        let calibration = match &self.correction {
            Some(c) => Calibration {
                input_floor: calibration.input_floor * c.floor_ratio,
                ..calibration
            }
            .validate()?,
            None => calibration,
        };
        ProcessingParameters::positive(calibration, roll, photo)
    }
}
