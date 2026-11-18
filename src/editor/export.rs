use super::output::filename;
pub(super) use super::output::{Collision, Kind, Outcome, Request, Summary};
#[cfg(test)]
use super::output::{Input, Photo, PhotoRequest, Services, destination, numbered, require_safe};
use super::*;
use crate::cancellation::Cancellation;

#[cfg(test)]
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

#[cfg(test)]
mod batch_tests;
#[cfg(test)]
mod tests;

#[derive(Default)]

pub(super) struct State {
    pub operation: Option<Operation>,
    pub form: Option<Form>,
    #[cfg(test)]
    pub summary: Option<Summary>,
}

/// The action retains cancellation feedback until its continuation acknowledges.
pub(super) struct Operation {
    #[cfg(test)]
    pub activity: ActivityReporter,
    pub cancelled: Cancellation,
    task: Option<async_executor::Task<()>>,
}

impl Drop for Operation {
    fn drop(&mut self) {
        self.cancelled.cancel();
    }
}

impl Editor {
    pub(super) fn export_photos(&mut self, request: Request, cx: &UiContext<'_, Self>) {
        let cancelled = Cancellation::default();
        let activity =
            self.activities
                .start(ActivityOptions::new("Exporting…").modal().cancellable(
                    "Cancel export",
                    "Cancelling export…",
                    cancelled.clone(),
                ));
        self.document.export.operation = Some(Operation {
            #[cfg(test)]
            activity: activity.reporter(),
            cancelled: cancelled.clone(),
            task: None,
        });
        self.status = "Exporting…".into();
        #[cfg(test)]
        {
            self.document.export.summary = None;
        }
        let total = request.photos.len();
        log::info!(total; "export_started");

        let services = self.output_services.clone();
        let retire = cancelled.cancel_on_drop();
        let Some(task) = cx.spawn(move |cx| async move {
            let _retire = retire;
            let summary = match services {
                Some(services) => {
                    services
                        .export_batch(request, cancelled, |completed, total| {
                            activity.progress(completed, total);
                        })
                        .await
                }
                None => Summary {
                    total,
                    stopped: true,
                    outcomes: vec![Outcome::Failed {
                        path: PathBuf::new(),
                        error: "Export services unavailable".into(),
                    }],
                },
            };
            drop(activity);
            cx.update(|editor| {
                log::info!(
                    total = summary.total,
                    exported = summary.exported(),
                    failed = summary.failed(),
                    stopped = summary.stopped;
                    "export_finished"
                );

                for outcome in &summary.outcomes {
                    if let Outcome::Failed { error, .. } = outcome {
                        log::warn!(
                            error = error.as_str();
                            "export_photo_failed"
                        );
                    }
                }
                editor.document.export.operation = None;
                if editor.interaction_blocked() {
                    // A captured batch cannot finish a newer foreground operation.
                    if summary.failed() > 0 {
                        editor.present_export_summary(summary);
                    }
                    return;
                }
                if summary.total == 1 && summary.exported() == 1 {
                    if let Outcome::Exported(path) = &summary.outcomes[0] {
                        editor.status = format!("Exported {}", path.display());
                    }

                    editor.present_export_summary(summary);
                } else if summary.total == 1 && summary.failed() == 1 {
                    if let Outcome::Failed { error, .. } = &summary.outcomes[0] {
                        editor.status = error.clone();
                        editor.export_failed(error.clone());
                    }
                } else if summary.stopped && summary.outcomes.is_empty() {
                    editor.status = "Export cancelled".into();
                } else {
                    editor.status = summary.text();
                    editor.present_export_summary(summary);
                }
            });
        }) else {
            self.document.export.operation = None;
            self.status = "Export cancelled".into();
            return;
        };
        self.document.export.operation.as_mut().unwrap().task = Some(task);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scope {
    Selected,
    All,
}

pub(super) struct Form {
    pub draft: Draft,
    _workspace: workspace_input::WorkspaceBlock,
}

impl Form {
    pub fn new(draft: Draft, input: &workspace_input::WorkspaceInput) -> Self {
        Self {
            draft,
            _workspace: input.block(),
        }
    }
}

pub(super) struct Draft {
    pub scope: Scope,
    pub folder: PathBuf,
    pub kind: Kind,
    pub target: OutputTarget,
    pub precision: ExrPrecision,
    pub suffix: String,
    pub collision: Collision,

    pub error: Option<String>,
}

impl Draft {
    pub fn new(target: OutputTarget, folder: PathBuf) -> Self {
        Self {
            scope: Scope::Selected,
            folder,
            kind: Kind::Tiff,
            target,
            precision: ExrPrecision::Half,
            suffix: String::new(),
            collision: Collision::KeepBoth,

            error: None,
        }
    }

    fn output(&self) -> output::Options {
        output::Options {
            folders: output::Folders::One(self.folder.clone()),
            format: self.format(),
            suffix: self.suffix.clone(),
            collision: self.collision,
        }
    }

    pub fn format(&self) -> OutputFormat {
        match self.kind {
            Kind::Tiff => OutputFormat::Tiff(self.target),
            Kind::Exr => OutputFormat::Exr(self.precision),
        }
    }
}

impl Editor {
    pub(super) fn export_ids(&self, scope: Scope) -> Vec<SourceId> {
        self.document
            .state
            .project
            .order
            .iter()
            .copied()
            .filter(|id| scope == Scope::All || self.document.roll_selection.contains(*id))
            .collect()
    }

    pub(super) fn capture_export(&self, draft: &Draft) -> Result<Request, Error> {
        self.capture_output(&draft.output(), self.export_ids(draft.scope))
    }
}

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
                let choose = ui::TextAction::new("Choose…");
                let available =
                    (ui.available_width() - choose.width(ui) - ui.spacing().item_spacing.x).max(1.);
                let size = egui::vec2(available, ui::CONTROL_HEIGHT);
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
            ui::labelled_row(ui, "Format", None, |ui, label| {
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
            ui::labelled_row(ui, "Target", None, |ui, label| {
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
                ui::labelled_row(ui, "Precision", None, |ui, label| {
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
                ui::labelled_row(ui, "Bit depth", None, |ui, _| {
                    ui.label("16-bit RGB");
                });
                ui::labelled_row(ui, "Compression", None, |ui, _| {
                    ui.label("Lossless · Deflate");
                });
            }
            ui.add_space(8.);
            ui.strong("Files");
            ui::labelled_row(ui, "Filename", None, |ui, _| {
                ui.label("Original filename");
            });
            ui::labelled_row(ui, "Suffix", None, |ui, label| {
                ui.add(egui::TextEdit::singleline(&mut draft.suffix).desired_width(240.))
                    .labelled_by(label);
            });
            if let Ok(name) = filename(
                std::path::Path::new("DSC01234.RAW"),
                &draft.suffix,
                draft.kind,
            ) {
                ui.small(format!("Example: {}", name.display()));
            }
            ui::labelled_row(ui, "Existing files", None, |ui, label| {
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
        let export = ui::TextAction::new(&format!(
            "Export {count} {}",
            if count == 1 { "photo" } else { "photos" }
        ))
        .primary()
        .enabled(count > 0 && !draft.folder.as_os_str().is_empty())
        .show(ui);
        intent.export = export.clicked();
        let cancel = ui::TextAction::new("Cancel").show(ui);
        intent.close = cancel.clicked();
        ui::focus_default(ui, &cancel);
    });
    intent
}

impl Editor {
    pub(super) fn open_export_dialog(&mut self) {
        if self.workspace_input.is_blocked() {
            return;
        }
        if self.editing() {
            self.status = "Finish or cancel the current edit before exporting".into();
            return;
        }
        let folder = self.preferences.export_folder.clone().unwrap_or_default();
        self.document.export.form = Some(Form::new(
            Draft::new(self.document.target(), folder),
            &self.workspace_input,
        ));
    }

    pub(super) fn export_dialogs(
        &mut self,
        context: &egui::Context,
        escape: bool,
        cx: &UiContext<'_, Self>,
    ) {
        if let Some(mut modal) = self.document.export.form.take() {
            let draft = &mut modal.draft;
            let selected = self.export_ids(Scope::Selected).len();
            let all = self.export_ids(Scope::All).len();
            let response = ui::modal(context, egui::Id::new("export-photos"), |ui| {
                ui.set_width((context.content_rect().width() - 80.).clamp(1., 500.));
                form(ui, draft, selected, all)
            });
            if response.inner.close || ui::escape_requested(&response, escape) {
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
                    self.request_export_replacement(modal, cx);
                } else {
                    self.accept_export(modal, cx);
                }
            } else {
                self.document.export.form = Some(modal);
            }
        }
    }

    fn present_export_summary(&mut self, summary: Summary) {
        #[cfg(test)]
        {
            self.document.export.summary = Some(summary.clone());
        }
        drop(self.dialogs.show(move |context, id, escape| {
            let response = ui::modal(context, id, |ui| {
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
                                Outcome::Failed { path, error } => {
                                    ui.label(format!("{}: {error}", path.display()));
                                }
                                Outcome::Skipped(path) => {
                                    ui.label(format!("Skipped {}", path.display()));
                                }
                                _ => {}
                            }
                        }
                    });
                let mut reveal = None;
                if let Some(Outcome::Exported(path)) = summary
                    .outcomes
                    .iter()
                    .find(|o| matches!(o, Outcome::Exported(_)))
                    && ui::TextAction::new("Show in Finder").show(ui).clicked()
                {
                    reveal = Some(path.clone());
                }
                let ok = ui::TextAction::new("OK").show(ui);
                ui::focus_default(ui, &ok);
                (ok.clicked(), reveal)
            });
            if let Some(path) = &response.inner.1 {
                reveal_file(path.clone());
            }
            (response.inner.0 || ui::escape_requested(&response, escape)).then_some(())
        }));
    }

    pub(super) fn request_export_replacement(&mut self, modal: Form, cx: &UiContext<'_, Self>) {
        let detail = modal.draft.folder.display().to_string();
        let response = self.dialogs.show(move |context, id, escape| {
            ui::DecisionDialog::new("Replace existing exported files?",
                "Existing files with the generated names in the destination folder will be replaced. Completed replacements cannot be undone.",
                "Replace and export").warning().detail(&detail).show(context, id, escape)
        });
        self.document.dialog_action = cx.spawn(move |cx| async move {
            let Ok(choice) = response.await else { return };
            cx.update(|editor| {
                if choice == ui::DialogChoice::Confirm {
                    editor.accept_export(modal, &cx);
                } else {
                    editor.document.export.form = Some(modal);
                }
            });
        });
    }

    fn accept_export(&mut self, mut modal: Form, cx: &UiContext<'_, Self>) {
        match self.capture_export(&modal.draft) {
            Ok(request) => {
                self.preferences.export_folder = Some(modal.draft.folder);
                self.persist_preferences();
                self.export_photos(request, cx);
            }
            Err(error) => {
                modal.draft.error = Some(error.to_string());
                self.document.export.form = Some(modal);
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

impl Editor {
    pub(super) fn export_failed(&mut self, details: String) {
        drop(self.dialogs.report(dialogs::Message {
            severity: super::ui::Severity::Error,
            title: "Could not export image",
            explanation: "Nothing was exported. The project and any existing destination file are unchanged. Review the details, then try exporting again.",
            details,
        }));
    }
}
