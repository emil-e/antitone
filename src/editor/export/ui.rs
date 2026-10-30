use super::{
    Editor, UiContext,
    batch::{self, Collision, Draft, Kind, Scope},
    presentation,
};
use crate::{export::ExrPrecision, rendering::OutputTarget};
use std::path::PathBuf;

#[derive(Default)]
struct Intent {
    close: bool,
    choose: bool,
    export: bool,
}

fn form(ui: &mut egui::Ui, draft: &mut Draft, selected: usize, all: usize) -> Intent {
    let mut intent = Intent::default();
    let form_inputs = (
        draft.scope,
        draft.folder.clone(),
        draft.kind,
        draft.target,
        draft.precision,
        draft.suffix.clone(),
        draft.collision,
    );
    ui.heading("Export photos");
    ui.add_space(8.);
    let height = (ui.ctx().content_rect().height() - 190.).max(80.);
    egui::ScrollArea::vertical()
        .max_height(height)
        .show(ui, |ui| {
            ui.strong("Photos");
            ui.horizontal(|ui| {
                ui.selectable_value(
                    &mut draft.scope,
                    Scope::Selected,
                    format!("Selected photos ({selected})"),
                );
                ui.selectable_value(&mut draft.scope, Scope::All, format!("All photos ({all})"));
            });
            ui.add_space(8.);
            ui.strong("Destination");
            ui.horizontal(|ui| {
                let choose = presentation::TextAction::new("Choose…");
                let available =
                    (ui.available_width() - choose.width(ui) - ui.spacing().item_spacing.x).max(1.);
                let size = egui::vec2(available, presentation::CONTROL_HEIGHT);
                ui.allocate_ui_with_layout(
                    size,
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_min_size(size);
                        ui.add(
                            egui::Label::new(if draft.folder.as_os_str().is_empty() {
                                "Choose a folder".into()
                            } else {
                                draft.folder.display().to_string()
                            })
                            .truncate(),
                        )
                        .on_hover_text(draft.folder.display().to_string());
                    },
                );
                intent.choose = choose.show(ui).clicked();
            });
            ui.add_space(8.);
            ui.strong("Output");
            presentation::labelled_row(ui, "Format", None, |ui, label| {
                egui::ComboBox::from_id_salt("export-format")
                    .width(240.)
                    .selected_text(match draft.kind {
                        Kind::Tiff => "TIFF",
                        Kind::Exr => "EXR",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut draft.kind, Kind::Tiff, "TIFF");
                        ui.selectable_value(&mut draft.kind, Kind::Exr, "EXR");
                    })
                    .response
                    .labelled_by(label);
            });
            presentation::labelled_row(ui, "Target", None, |ui, label| {
                ui.add_enabled_ui(draft.kind == Kind::Tiff, |ui| {
                    egui::ComboBox::from_id_salt("export-target")
                        .width(240.)
                        .selected_text(if draft.kind == Kind::Exr {
                            "Scene-linear AP0"
                        } else {
                            match draft.target {
                                OutputTarget::Srgb => "sRGB",
                                OutputTarget::DisplayP3 => "Display P3",
                            }
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut draft.target, OutputTarget::Srgb, "sRGB");
                            ui.selectable_value(
                                &mut draft.target,
                                OutputTarget::DisplayP3,
                                "Display P3",
                            );
                        })
                        .response
                        .labelled_by(label);
                });
            });
            if draft.kind == Kind::Exr {
                presentation::labelled_row(ui, "Precision", None, |ui, label| {
                    egui::ComboBox::from_id_salt("export-precision")
                        .width(240.)
                        .selected_text(match draft.precision {
                            ExrPrecision::Half => "Half float · 16-bit",
                            ExrPrecision::Full => "Full float · 32-bit",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut draft.precision,
                                ExrPrecision::Half,
                                "Half float · 16-bit",
                            );
                            ui.selectable_value(
                                &mut draft.precision,
                                ExrPrecision::Full,
                                "Full float · 32-bit",
                            );
                        })
                        .response
                        .labelled_by(label);
                });
                ui.small("Creative look and display rendering are not applied.");
            } else {
                presentation::labelled_row(ui, "Bit depth", None, |ui, _| {
                    ui.label("16-bit RGB");
                });
                presentation::labelled_row(ui, "Compression", None, |ui, _| {
                    ui.label("Lossless · Deflate");
                });
            }
            ui.add_space(8.);
            ui.strong("Files");
            presentation::labelled_row(ui, "Filename", None, |ui, _| {
                ui.label("Original filename");
            });
            presentation::labelled_row(ui, "Suffix", None, |ui, label| {
                ui.add(egui::TextEdit::singleline(&mut draft.suffix).desired_width(240.))
                    .labelled_by(label);
            });
            if let Ok(name) = batch::filename(
                std::path::Path::new("DSC01234.RAW"),
                &draft.suffix,
                draft.kind,
            ) {
                ui.small(format!("Example: {}", name.display()));
            }
            presentation::labelled_row(ui, "Existing files", None, |ui, label| {
                egui::ComboBox::from_id_salt("export-existing")
                    .width(240.)
                    .selected_text(match draft.collision {
                        Collision::KeepBoth => "Keep both",
                        Collision::Skip => "Skip",
                        Collision::Replace => "Replace…",
                        #[cfg(test)]
                        Collision::Refuse => "Refuse",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut draft.collision, Collision::KeepBoth, "Keep both");
                        ui.selectable_value(&mut draft.collision, Collision::Skip, "Skip");
                        ui.selectable_value(&mut draft.collision, Collision::Replace, "Replace…");
                    })
                    .response
                    .labelled_by(label);
            });
            let inputs = (
                draft.scope,
                draft.folder.clone(),
                draft.kind,
                draft.target,
                draft.precision,
                draft.suffix.clone(),
                draft.collision,
            );
            if inputs != form_inputs {
                draft.error = None;
            }
            if let Some(error) = &draft.error {
                ui.label(error);
            }
        });
    ui.add_space(12.);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let count = if draft.scope == Scope::Selected {
            selected
        } else {
            all
        };
        let export = presentation::TextAction::new(&format!(
            "Export {count} {}",
            if count == 1 { "photo" } else { "photos" }
        ))
        .primary()
        .enabled(count > 0 && !draft.folder.as_os_str().is_empty())
        .show(ui);
        intent.export = export.clicked();
        let cancel = presentation::TextAction::new("Cancel").show(ui);
        intent.close = cancel.clicked();
        presentation::focus_default(ui, &cancel);
    });
    intent
}

impl Editor {
    pub(in crate::editor) fn export_modal(&self) -> bool {
        self.document.export.draft.is_some() || self.document.export.summary.is_some()
    }

    pub(in crate::editor) fn open_export_dialog(&mut self) {
        if self.dust_search_pending() {
            self.status = "Wait for donor selection before exporting".into();
            return;
        }
        let folder = self.preferences.export_folder.clone().unwrap_or_default();
        self.document.export.draft = Some(Draft::new(self.document.target, folder));
    }

    pub(in crate::editor) fn export_dialogs(
        &mut self,
        context: &egui::Context,
        escape: bool,
        cx: &UiContext<'_, Self>,
    ) {
        if let Some(mut draft) = self.document.export.draft.take() {
            if draft.confirming_replace {
                let choice = presentation::DecisionDialog::new("Replace existing exported files?", "Existing files with the generated names in the destination folder will be replaced. Completed replacements cannot be undone.", "Replace and export")
                    .warning().detail(&draft.folder.display().to_string()).show(context, egui::Id::new("confirm-batch-replace"), escape);
                match choice {
                    Some(presentation::DialogChoice::Confirm) => self.accept_export(draft, cx),
                    Some(presentation::DialogChoice::Cancel) => {
                        draft.confirming_replace = false;
                        self.document.export.draft = Some(draft);
                    }
                    _ => self.document.export.draft = Some(draft),
                }
                return;
            }
            let selected = self.export_ids(Scope::Selected).len();
            let all = self.export_ids(Scope::All).len();
            let response = presentation::modal(context, egui::Id::new("export-photos"), |ui| {
                ui.set_width((context.content_rect().width() - 80.).clamp(1., 500.));
                form(ui, &mut draft, selected, all)
            });
            if response.inner.close || presentation::escape_requested(&response, escape) {
                return;
            }
            if response.inner.choose {
                let mut picker = rfd::FileDialog::new().set_title("Export destination");
                if !draft.folder.as_os_str().is_empty() {
                    picker = picker.set_directory(&draft.folder);
                }
                if let Some(folder) =
                    crate::platform::menu::with_native_dialog(|| picker.pick_folder())
                {
                    draft.error = None;
                    draft.folder = folder;
                }
            }
            if response.inner.export {
                if draft.collision == Collision::Replace {
                    draft.confirming_replace = true;
                    self.document.export.draft = Some(draft);
                } else {
                    self.accept_export(draft, cx);
                }
            } else {
                self.document.export.draft = Some(draft);
            }
        } else if let Some(summary) = self.document.export.summary.take() {
            let response = presentation::modal(context, egui::Id::new("export-summary"), |ui| {
                ui.heading(if summary.stopped {
                    "Export stopped"
                } else {
                    "Export complete"
                });
                ui.label(summary.text());
                egui::ScrollArea::vertical()
                    .max_height(180.)
                    .show(ui, |ui| {
                        for outcome in &summary.outcomes {
                            match outcome {
                                batch::Outcome::Failed { path, error } => {
                                    ui.label(format!("{}: {error}", path.display()));
                                }
                                batch::Outcome::Skipped(path) => {
                                    ui.label(format!("Skipped {}", path.display()));
                                }
                                _ => {}
                            }
                        }
                    });
                let mut reveal = None;
                if let Some(batch::Outcome::Exported(path)) = summary
                    .outcomes
                    .iter()
                    .find(|o| matches!(o, batch::Outcome::Exported(_)))
                    && presentation::TextAction::new("Show in Finder")
                        .show(ui)
                        .clicked()
                {
                    reveal = Some(path.clone());
                }
                let ok = presentation::TextAction::new("OK").show(ui);
                presentation::focus_default(ui, &ok);
                (ok.clicked(), reveal)
            });
            if let Some(path) = &response.inner.1 {
                reveal_file(path.clone());
            }
            if !response.inner.0 && !presentation::escape_requested(&response, escape) {
                self.document.export.summary = Some(summary);
            }
        }
    }

    fn accept_export(&mut self, mut draft: Draft, cx: &UiContext<'_, Self>) {
        match self.capture_export(&draft) {
            Ok(request) => {
                self.preferences.export_folder = Some(draft.folder);
                self.persist_preferences();
                self.export_photos(request, cx);
            }
            Err(error) => {
                draft.error = Some(error.to_string());
                draft.confirming_replace = false;
                self.document.export.draft = Some(draft);
            }
        }
    }
}

fn reveal_file(path: PathBuf) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("/usr/bin/open")
            .arg("-R")
            .arg(path)
            .spawn();
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
    }
}
