use super::presentation::TextAction;
use super::*;

#[cfg(test)]
#[path = "editor_files_tests.rs"]
mod tests;

// Picker hints only; decoded sensor data and metadata determine compatibility.
pub(super) const RAW_EXTENSIONS: &[&str] = &[
    "3fr", "arw", "bay", "cr2", "cr3", "crw", "dcr", "dng", "erf", "fff", "iiq", "kdc", "mef",
    "mos", "mrw", "nef", "nrw", "orf", "pef", "raf", "raw", "rw2", "rwl", "sr2", "srf", "srw",
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum FileAction {
    New,
    Open,
    Add,
    Save,
    SaveAs,
    Close,
}
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Transition {
    New,
    Open(PathBuf),
    Close,
}

impl Editor {
    pub(super) fn perform_transition(&mut self, transition: Transition, context: &egui::Context) {
        match transition {
            Transition::New => self.start_document(DocumentWork::New),
            Transition::Open(path) => self.start_document(DocumentWork::Open(path)),
            Transition::Close => {
                self.allow_close = true;
                context.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
    pub(super) fn guard_transition(&mut self, transition: Transition, context: &egui::Context) {
        self.sync_document();
        if self.document.dirty() {
            self.transition = Some(transition);
        } else {
            self.perform_transition(transition, context);
        }
    }
    pub(super) fn queue_save(
        &mut self,
        path: PathBuf,
        policy: crate::project::Save,
        after: Option<Transition>,
    ) {
        self.sync_document();
        self.after_save = after;
        self.start_document(DocumentWork::Save(self.document.clone(), path, policy));
        if self.document_pending.is_none() {
            self.transition = self.after_save.take();
        }
    }
    fn save_dialog(&mut self, save_as: bool, after: Option<Transition>) {
        if !save_as && let Some(path) = self.document.path.clone() {
            self.queue_save(path, crate::project::Save::Replace, after);
            return;
        }
        let name = self
            .document
            .path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled.antitone.json".into());
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save Antitone project")
            .set_file_name(name)
            .add_filter("Antitone project", &["json"]);
        if let Some(parent) = self.document.path.as_ref().and_then(|path| path.parent()) {
            dialog = dialog.set_directory(parent);
        }
        if let Some(path) = dialog.save_file() {
            if path.exists() && self.document.path.as_ref() != Some(&path) {
                self.project_overwrite = Some((path, after));
            } else {
                let policy = if path.exists() {
                    crate::project::Save::Replace
                } else {
                    crate::project::Save::New
                };
                self.queue_save(path, policy, after);
            }
        }
    }
    pub(super) fn file_action(&mut self, action: FileAction, context: &egui::Context) {
        if self.crop_edit.is_some() {
            self.status = "Apply or cancel the crop before changing or saving the document".into();
            return;
        }
        match action {
            FileAction::New => self.guard_transition(Transition::New, context),
            FileAction::Close => self.guard_transition(Transition::Close, context),
            FileAction::Open => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Open Antitone project")
                    .add_filter("Antitone project", &["json"])
                    .pick_file()
                {
                    self.guard_transition(Transition::Open(path), context);
                }
            }
            FileAction::Add => {
                if let Some(paths) = rfd::FileDialog::new()
                    .add_filter("Camera RAW", RAW_EXTENSIONS)
                    .pick_files()
                {
                    self.add_raws(paths);
                }
            }
            FileAction::Save => self.save_dialog(false, None),
            FileAction::SaveAs => self.save_dialog(true, None),
        }
    }
    pub(super) fn document_dialogs(&mut self, context: &egui::Context) {
        if let Some(transition) = self.transition.clone() {
            egui::Window::new("Save project changes?")
                .collapsible(false)
                .resizable(false)
                .show(context, |ui| {
                    ui.label("Save this project's changes before continuing?");
                    ui.horizontal(|ui| {
                        if TextAction::new("Cancel").show(ui).clicked() {
                            self.transition = None;
                        }
                        if TextAction::new("Discard").show(ui).clicked() {
                            self.transition = None;
                            self.perform_transition(transition.clone(), context);
                        }
                        if TextAction::new("Save").show(ui).clicked() {
                            self.transition = None;
                            self.save_dialog(false, Some(transition.clone()));
                        }
                    });
                });
        }
        if let Some((path, after)) = self.project_overwrite.clone() {
            egui::Window::new("Replace existing project?")
                .collapsible(false)
                .resizable(false)
                .show(context, |ui| {
                    ui.label(path.display().to_string());
                    ui.horizontal(|ui| {
                        if TextAction::new("Cancel").show(ui).clicked() {
                            self.project_overwrite = None;
                        }
                        if TextAction::new("Replace").show(ui).clicked() {
                            self.project_overwrite = None;
                            self.queue_save(
                                path.clone(),
                                crate::project::Save::Replace,
                                after.clone(),
                            );
                        }
                    });
                });
        }
    }
}
