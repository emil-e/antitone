use super::*;
mod interaction;
mod operation;
pub(super) use interaction::FlatTool;
#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;
mod worker;
use crate::{
    gpu::{flat::GpuFlatField, source::SourceGeometry},
    project::FlatFieldSettings,
};
pub(super) use worker::CorrectionInput;

pub(super) struct Correction {
    pub settings: FlatFieldSettings,
    pub gpu: Arc<GpuFlatField>,
    pub(super) maximum_gain: f64,
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
                let continuous = source.development.lens_correction.is_some();
                crate::gpu::parameters::CameraMatrix::require_source(matrix, continuous)?;
                gpu.validate_matrix(
                    matrix,
                    if continuous {
                        crate::processing::MAX_SOURCE_GAIN
                    } else {
                        1.
                    },
                )?;
                SourceGeometry::new(source.development.active_area, [1, 1])?;
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

/// Prepared reference inputs owned by the Flat interaction.
pub(super) struct FlatEdit {
    pub services: Option<source::Services>,
    pub view: super::canvas::SourceView,
    pub region: Option<Coverage>,
    pub task: Option<async_executor::Task<()>>,
    pub activity: Option<TaskRequest>,
}

pub(super) struct FlatPreview {
    pub ready: Ready,
    pub region: Option<Rect>,
}

impl Editor {
    pub(super) fn flat_controls(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) {
        let editing = self
            .current_tool()
            .is_some_and(|tool| tool.is::<FlatTool>());
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
                    return;
                }
                ui.add_enabled_ui(available, |ui| {
                    if let Some(settings) = self.document.state.project.flat_field() {
                        let path = self.document.source_path(settings.reference).ok();
                        let mut enabled = settings.enabled;
                        if ui.add_enabled(!self.tool_interacting(), egui::Checkbox::new(&mut enabled, "Enabled")).changed() {
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
}
