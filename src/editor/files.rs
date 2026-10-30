use super::presentation::{DecisionDialog, DialogChoice};
use super::*;

#[cfg(test)]
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

/// Picker-extension hint: any listed RAW routes to scan import; contents decide compatibility.
pub(super) fn is_raw_path(path: &std::path::Path) -> bool {
    path.extension().is_some_and(|extension| {
        RAW_EXTENSIONS
            .iter()
            .any(|raw| extension.eq_ignore_ascii_case(raw))
    })
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
    pub(super) fn perform_transition(&mut self, transition: Transition, cx: &UiContext<'_, Self>) {
        match transition {
            Transition::New => self.start_document(Preparation::New, cx),
            Transition::Open(path) => self.start_document(Preparation::Open(path), cx),
            Transition::Close => {
                self.allow_close = true;
            }
        }
    }

    pub(super) fn guard_transition(&mut self, transition: Transition, cx: &UiContext<'_, Self>) {
        self.sync_document();
        if self.document.dirty() {
            self.transition = Some(transition);
        } else {
            self.perform_transition(transition, cx);
        }
    }

    pub(super) fn save_project(
        &mut self,
        path: PathBuf,
        policy: crate::project::Save,
        after: Option<Transition>,
        cx: &UiContext<'_, Self>,
    ) {
        if self.dust_search_pending() {
            self.status = "Wait for donor selection before saving".into();
            return;
        }
        self.sync_document();
        self.after_save = after;
        self.start_save(
            document_services::Write {
                document: self.document.capture(),
                path,
                policy,
            },
            cx,
        );
    }

    fn save_dialog(&mut self, save_as: bool, after: Option<Transition>, cx: &UiContext<'_, Self>) {
        if !save_as && let Some(path) = self.document.path.clone() {
            self.save_project(path, crate::project::Save::Replace, after, cx);
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
                self.save_project(path, policy, after, cx);
            }
        }
    }

    pub(super) fn file_action(&mut self, action: FileAction, cx: &UiContext<'_, Self>) {
        if self.dust_search_pending() {
            self.status = "Wait for donor selection before changing or saving the document".into();
            return;
        }
        if self.document.crop_edit().is_some() || self.document.auto_crop_edit().is_some() {
            self.status =
                "Apply or cancel the crop tool before changing or saving the document".into();
            return;
        }
        match action {
            FileAction::New => self.guard_transition(Transition::New, cx),
            FileAction::Close => self.guard_transition(Transition::Close, cx),
            FileAction::Open => {
                if let Some(path) = crate::platform::menu::with_native_dialog(|| {
                    rfd::FileDialog::new()
                        .set_title("Open Antitone Project")
                        .add_filter("Antitone project", &["antitone", "json"])
                        .pick_file()
                }) {
                    self.guard_transition(Transition::Open(path), cx);
                }
            }
            FileAction::Add => {
                if let Some(paths) = crate::platform::menu::with_native_dialog(|| {
                    rfd::FileDialog::new()
                        .set_title("Add Scans")
                        .add_filter("Camera RAW files", RAW_EXTENSIONS)
                        .pick_files()
                }) {
                    self.add_raws(paths, cx);
                }
            }
            FileAction::Remove => self.request_removal(),
            FileAction::Save => self.save_dialog(false, None, cx),
            FileAction::SaveAs => self.save_dialog(true, None, cx),
        }
    }

    pub(super) fn document_dialogs(
        &mut self,
        context: &egui::Context,
        escape_available: bool,
        cx: &UiContext<'_, Self>,
    ) {
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
                    self.perform_transition(transition, cx);
                }
                Some(DialogChoice::Confirm) => {
                    self.transition = None;
                    self.save_dialog(false, Some(transition), cx);
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
                    self.save_project(path, crate::project::Save::Replace, after, cx);
                }
                _ => {}
            }
        }
    }
}
