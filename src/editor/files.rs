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

#[derive(Default)]
pub(super) struct State {
    action: Option<Pending>,
}

struct Pending {
    closing: bool,
    task: async_executor::Task<()>,
}

impl State {
    pub fn pending(&self) -> bool {
        self.action
            .as_ref()
            .is_some_and(|action| !action.task.is_finished())
    }
    pub fn closing(&self) -> bool {
        self.pending() && self.action.as_ref().unwrap().closing
    }
}

pub(super) struct Destination {
    pub path: PathBuf,
    pub policy: crate::project::Save,
    pub confirm_replace: bool,
}

impl Editor {
    pub(super) fn perform_transition(&mut self, transition: Transition, cx: &UiContext<'_, Self>) {
        match transition {
            Transition::New => self.start_document(Preparation::New, cx),
            Transition::Open(path) => self.start_document(Preparation::Open(path), cx),
            Transition::Close => self.allow_close = true,
        }
    }

    pub(super) fn guard_transition(&mut self, transition: Transition, cx: &UiContext<'_, Self>) {
        if self.files.pending() {
            return;
        }
        if !self.document.dirty() {
            self.perform_transition(transition, cx);
            return;
        }
        let closing = transition == Transition::Close;
        let id = self.id;
        self.files.action = cx
            .spawn(move |cx| async move {
                guarded_transition(&cx, id, transition).await;
            })
            .map(|task| Pending { closing, task });
    }

    pub(super) fn save_project(
        &mut self,
        path: PathBuf,
        policy: crate::project::Save,
        after: Option<Transition>,
        cx: &UiContext<'_, Self>,
    ) {
        if self.files.pending() || self.dust_search_pending() {
            return;
        }
        let id = self.id;
        let response = self.start_save(
            document::Write {
                document: self.document.capture(),
                path,
                policy,
            },
            cx,
        );
        let closing = after == Some(Transition::Close);
        self.files.action = cx
            .spawn(move |cx| async move {
                let Ok(success) = response.await else { return };
                if let Some(transition) = after {
                    if success {
                        finish_transition(&cx, id, transition).await;
                    } else {
                        guarded_transition(&cx, id, transition).await;
                    }
                }
            })
            .map(|task| Pending { closing, task });
    }

    fn save_destination(&self, save_as: bool) -> Option<Destination> {
        if !save_as && let Some(path) = self.document.path.clone() {
            return Some(Destination {
                path,
                policy: crate::project::Save::Replace,
                confirm_replace: false,
            });
        }
        let name = project_save_name(self.document.path.as_deref());
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save Antitone project")
            .set_file_name(name)
            .add_filter("Antitone project", &["antitone"]);
        if let Some(parent) = self.document.path.as_ref().and_then(|path| path.parent()) {
            dialog = dialog.set_directory(parent);
        }
        let path = crate::platform::menu::with_native_dialog(|| dialog.save_file())?;
        let exists = path.exists();
        Some(Destination {
            confirm_replace: exists && self.document.path.as_ref() != Some(&path),
            policy: if exists {
                crate::project::Save::Replace
            } else {
                crate::project::Save::New
            },
            path,
        })
    }

    fn save_dialog(&mut self, save_as: bool, cx: &UiContext<'_, Self>) {
        if self.files.pending() {
            return;
        }
        let Some(destination) = self.save_destination(save_as) else {
            return;
        };
        self.save_destination_action(destination, None, cx);
    }

    pub(super) fn save_destination_action(
        &mut self,
        destination: Destination,
        after: Option<Transition>,
        cx: &UiContext<'_, Self>,
    ) {
        if self.files.pending() {
            return;
        }
        if !destination.confirm_replace {
            self.save_project(destination.path, destination.policy, after, cx);
            return;
        }

        let id = self.id;
        let closing = after == Some(Transition::Close);
        self.files.action = cx
            .spawn(move |cx| async move {
                match save_to(&cx, id, destination).await {
                    Some(true) => {
                        if let Some(transition) = after {
                            finish_transition(&cx, id, transition).await;
                        }
                    }
                    Some(false) => {
                        if let Some(transition) = after {
                            guarded_transition(&cx, id, transition).await;
                        }
                    }
                    None => {}
                }
            })
            .map(|task| Pending { closing, task });
    }

    pub(super) fn file_action(&mut self, action: FileAction, cx: &UiContext<'_, Self>) {
        if self.files.pending() {
            return;
        }
        if self.dust_search_pending() {
            self.status = "Wait for donor selection before changing or saving the document".into();
            return;
        }
        if self.tool_interacting() || self.document.visit.masks.editing() {
            self.status = "Finish the active interaction before a file action".into();
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
            FileAction::Remove => self.request_removal(cx),
            FileAction::Save => self.save_dialog(false, cx),
            FileAction::SaveAs => self.save_dialog(true, cx),
        }
    }
}

async fn guarded_transition(cx: &UiContext<'_, Editor>, id: u64, transition: Transition) {
    loop {
        let response = cx.update(|editor| {
            if editor.id != id { return None; }
            let explanation = match &transition {
                Transition::New => "Creating a new project without saving will discard unsaved edits in this project.",
                Transition::Open(_) => "Opening another project without saving will discard unsaved edits in this project.",
                Transition::Close => "Closing Antitone without saving will discard unsaved edits in this project.",
            };
            Some(editor.dialogs.show(move |context, dialog, escape| {
                DecisionDialog::new("Save changes to this project?", explanation, "Save changes")
                    .detail("Your RAW files are not changed.").alternative("Discard changes")
                    .show(context, dialog, escape)
            }))
        }).flatten();
        let Some(response) = response else { return };
        match response.await {
            Ok(DialogChoice::Alternative) => {}
            Ok(DialogChoice::Confirm) => {
                let destination = cx
                    .update(|editor| {
                        (editor.id == id)
                            .then(|| editor.save_destination(false))
                            .flatten()
                    })
                    .flatten();
                let Some(destination) = destination else {
                    return;
                };
                match save_to(cx, id, destination).await {
                    Some(true) => {}
                    Some(false) => continue,
                    None => return,
                }
            }
            _ => return,
        }
        finish_transition(cx, id, transition).await;
        return;
    }
}

async fn finish_transition(cx: &UiContext<'_, Editor>, id: u64, transition: Transition) {
    let changes = cx.read(|editor| editor.dialogs.changes());
    loop {
        let changed = changes.listen();
        if cx.read(|editor| editor.id != id) {
            return;
        }
        if !cx.read(|editor| editor.dialogs.is_open()) {
            break;
        }
        changed.await;
    }
    cx.update(|editor| {
        if editor.id == id {
            editor.perform_transition(transition, cx);
        }
    });
}

async fn save_to(cx: &UiContext<'_, Editor>, id: u64, destination: Destination) -> Option<bool> {
    if destination.confirm_replace {
        let detail = destination.path.display().to_string();
        let response = cx.update(|editor| editor.dialogs.show(move |context, dialog, escape| {
            DecisionDialog::new("Replace this project file?",
                "A project already exists at this location. Replacing it overwrites the saved project, not its RAW files.",
                "Replace project").warning().detail(&detail).show(context, dialog, escape)
        }))?;
        if response.await != Ok(DialogChoice::Confirm) {
            return None;
        }
    }
    let response = cx
        .update(|editor| {
            if editor.id != id || editor.dust_search_pending() {
                return None;
            }

            Some(editor.start_save(
                document::Write {
                    document: editor.document.capture(),
                    path: destination.path,
                    policy: destination.policy,
                },
                cx,
            ))
        })
        .flatten()?;
    response.await.ok()
}
