use super::presentation::{DecisionDialog, DialogChoice};
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
    Remove,
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

fn project_save_name(path: Option<&std::path::Path>) -> String {
    let mut name = path
        .and_then(|path| path.file_name())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("Untitled"));
    if name
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
    {
        name.set_extension("");
    }
    if !name
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("antitone"))
    {
        name.as_mut_os_string().push(".antitone");
    }
    name.to_string_lossy().into_owned()
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
        let name = project_save_name(self.document.path.as_deref());
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save Antitone project")
            .set_file_name(name)
            .add_filter("Antitone project", &["antitone"]);
        if let Some(parent) = self.document.path.as_ref().and_then(|path| path.parent()) {
            dialog = dialog.set_directory(parent);
        }
        if let Some(path) = crate::platform::menu::with_native_dialog(|| dialog.save_file()) {
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
        if self.crop_edit.is_some() || self.auto_crop_edit.is_some() {
            self.status =
                "Apply or cancel the crop tool before changing or saving the document".into();
            return;
        }
        match action {
            FileAction::New => self.guard_transition(Transition::New, context),
            FileAction::Close => self.guard_transition(Transition::Close, context),
            FileAction::Open => {
                if let Some(path) = crate::platform::menu::with_native_dialog(|| {
                    rfd::FileDialog::new()
                        .set_title("Open Antitone Project")
                        .add_filter("Antitone project", &["antitone", "json"])
                        .pick_file()
                }) {
                    self.guard_transition(Transition::Open(path), context);
                }
            }
            FileAction::Add => {
                if let Some(paths) = crate::platform::menu::with_native_dialog(|| {
                    rfd::FileDialog::new()
                        .set_title("Add Scans")
                        .add_filter("Camera RAW files", RAW_EXTENSIONS)
                        .pick_files()
                }) {
                    self.add_raws(paths);
                }
            }
            FileAction::Remove => self.request_removal(),
            FileAction::Save => self.save_dialog(false, None),
            FileAction::SaveAs => self.save_dialog(true, None),
        }
    }

    pub(super) fn document_dialogs(&mut self, context: &egui::Context, escape_available: bool) {
        if let Some(transition) = self.transition.clone() {
            let explanation = match &transition {
                Transition::New => {
                    "Creating a new project without saving will discard unsaved edits in this project."
                }
                Transition::Open(_) => {
                    "Opening another project without saving will discard unsaved edits in this project."
                }
                Transition::Close => {
                    "Closing Antitone without saving will discard unsaved edits in this project."
                }
            };
            let choice =
                DecisionDialog::new("Save changes to this project?", explanation, "Save changes")
                    .detail("Your RAW files are not changed.")
                    .alternative("Discard changes")
                    .show(
                        context,
                        egui::Id::new("Save project changes?"),
                        escape_available,
                    );
            match choice {
                Some(DialogChoice::Cancel) => self.transition = None,
                Some(DialogChoice::Alternative) => {
                    self.transition = None;
                    self.perform_transition(transition, context);
                }
                Some(DialogChoice::Confirm) => {
                    self.transition = None;
                    self.save_dialog(false, Some(transition));
                }
                None => {}
            }
            // A save chooser can stage an overwrite; show that decision on the next frame.
            return;
        }
        if let Some((path, after)) = self.project_overwrite.clone() {
            let detail = path.display().to_string();
            let choice = DecisionDialog::new(
                "Replace this project file?",
                "A project already exists at this location. Replacing it overwrites the saved project, not its RAW files.",
                "Replace project",
            )
            .warning()
            .detail(&detail)
            .show(
                context,
                egui::Id::new("Replace existing project?"),
                escape_available,
            );
            match choice {
                Some(DialogChoice::Cancel) => self.project_overwrite = None,
                Some(DialogChoice::Confirm) => {
                    self.project_overwrite = None;
                    self.queue_save(path, crate::project::Save::Replace, after);
                }
                _ => {}
            }
        }
    }
}
