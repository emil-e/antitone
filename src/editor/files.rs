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
    open: Option<async_executor::Task<()>>,
}

struct Pending {
    closing: bool,
    task: async_executor::Task<()>,
}

impl State {
    pub fn opening(&self) -> bool {
        self.open.as_ref().is_some_and(|task| !task.is_finished())
    }

    pub fn pending(&self) -> bool {
        self.opening()
            || self
                .action
                .as_ref()
                .is_some_and(|action| !action.task.is_finished())
    }
    pub fn closing(&self) -> bool {
        self.action
            .as_ref()
            .is_some_and(|action| action.closing && !action.task.is_finished())
    }
}

pub(super) struct Destination {
    pub path: PathBuf,
    pub policy: crate::project::Save,
    pub confirm_replace: bool,
}

impl Editor {
    pub(super) fn add_raws(&mut self, paths: Vec<PathBuf>, cx: &UiContext<'_, Self>) {
        if paths.is_empty() {
            return;
        }
        let edit = match self.edit_document("Import photos") {
            Ok(edit) => edit,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        let mut document = edit.initial().as_ref().clone();
        let activity = self
            .activities
            .start(ActivityOptions::new("Importing RAWs…").modal());
        let progress = activity.reporter();
        self.document.importing = cx.spawn(move |cx| async move {
            let result = crate::background::run(move |cancelled| {
                let previews =
                    document.import_with_progress(&paths, &cancelled, |done, total| {
                        progress.progress(done, total)
                    })?;
                Ok((document, previews))
            })
            .await
            .map_err(|_| "Import worker panicked".to_owned())
            .and_then(|result: Result<_, Error>| result.map_err(|error| error.to_string()));
            drop(activity);
            cx.update(|editor| {
                editor.document.importing = None;
                if !editor.owns_document_edit(&edit) {
                    return;
                }
                let (mut document, previews) = match result {
                    Ok(result) => result,
                    Err(error) => {
                        editor.status = error;
                        return;
                    }
                };
                if document.project == editor.document.project {
                    return;
                }
                let input_changed = document.project.input != editor.document.project.input;
                document.capture_initialization(&editor.document.initialization);
                let mut edit = edit;
                edit.replace_snapshot(Arc::new(document));
                editor.commit_document(edit).expect("current import edit");
                editor.document.source_previews.extend(previews);
                editor.document.roll_selection.reconcile(
                    &editor.document.state.project.order,
                    editor.document.visit.current_source,
                );
                editor.status = "Photos added".into();
                if input_changed {
                    editor.reinterpret_input(&cx);
                }
                let load = if editor.document.visit.current_source.is_none() {
                    editor.document.project.order.first().copied()
                } else {
                    None
                };
                if let Some(id) = load
                    && let Ok(path) = editor.document.source_path(id)
                {
                    editor.load(path, &cx);
                }
            });
        });
    }

    pub(super) fn new_project(&mut self) {
        self.files.open = None;
        self.install_project(Document::default());
        self.tab = 0;
        self.preferences
            .sections
            .insert("roll-calibration".into(), true);
    }

    pub(super) fn open_project(&mut self, path: PathBuf, cx: &UiContext<'_, Self>) {
        let activity = self
            .activities
            .start(ActivityOptions::new("Opening project…").modal());
        self.files.open = cx.spawn(move |cx| async move {
            let result = crate::background::run(move |_| Document::open(&path))
                .await
                .map_err(|_| "Project reader panicked".to_owned())
                .and_then(|result| result.map_err(|error| error.to_string()));
            drop(activity);
            cx.update(|editor| match result {
                Ok(document) => {
                    editor.install_project(document);
                    editor.restore_correction(&cx);
                    if let Some(id) = editor.document.project.order.first().copied()
                        && let Ok(path) = editor.document.source_path(id)
                    {
                        editor.document.visit.current_source = Some(id);
                        editor.load(path, &cx);
                    }
                }
                Err(error) => editor.status = error,
            });
        });
    }

    fn install_project(&mut self, document: Document) {
        self.cancel_active_tool();
        self.document_pending = None;
        self.document = self.document.replacement(document);
        self.id = self.request_id();
        self.document.visit.view = self.id;
        self.source_epoch
            .store(self.id, std::sync::atomic::Ordering::Relaxed);
        self.canvas_frame = Default::default();
        self.remember_imported_look();
        self.invalidate_original();
        self.status = "Project ready".into();
    }

    pub(super) fn perform_transition(&mut self, transition: Transition, cx: &UiContext<'_, Self>) {
        match transition {
            Transition::New => self.new_project(),
            Transition::Open(path) => self.open_project(path, cx),
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
