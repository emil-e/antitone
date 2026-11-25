use super::*;
use crate::editor::external_edit as external;
use std::sync::Mutex;

struct HostInput {
    paths: Vec<PathBuf>,
    destination: Arc<dyn external::Destination>,
}

struct HostDestination {
    folder: PathBuf,
    published: Mutex<Option<(crate::project::Project, Vec<SourceId>)>>,
    reject: bool,
}

const PRESENTATION: external::Presentation = external::Presentation {
    action: "Return to Test Host",
    opening: "Opening host edit…",
    working: "Returning host edit…",
    completed: "Returned to test host",
};

impl external::Input for HostInput {
    fn presentation(&self) -> external::Presentation {
        PRESENTATION
    }

    fn prepare(
        self: Box<Self>,
        cancelled: &crate::cancellation::Cancellation,
        progress: &mut dyn FnMut(usize, usize),
    ) -> Result<external::Opened, Error> {
        let mut document = Document::default();
        document.import_with_progress(&self.paths, cancelled, progress)?;
        let ids = document.project.order.clone();
        let session = external::Session::new(&document, ids, self.destination)?;
        Ok(external::Opened { document, session })
    }
}

impl external::Destination for HostDestination {
    fn presentation(&self) -> external::Presentation {
        PRESENTATION
    }
    fn output(
        &self,
        _document: &Document,
        target: OutputTarget,
    ) -> Result<crate::editor::output::Options, Error> {
        Ok(crate::editor::output::Options::new(
            OutputFormat::Tiff(target).plan(),
            output::Folders::One(self.folder.clone()),
        ))
    }

    fn publish(&self, document: &Document, renders: &[external::Rendered]) -> Result<(), Error> {
        let path = document
            .path
            .as_ref()
            .expect("publish receives a saved document");
        assert_eq!(path, &self.folder.join("host.antitone"));
        let saved = Document::open(path).expect("the project is on disk before publication");
        assert_eq!(saved.project.roll, document.project.roll);
        assert!(!document.dirty());
        if self.reject {
            return Err(Error::Project("host rejected the update".into()));
        }
        for render in renders {
            assert!(render.path.is_file(), "the host receives completed renders");
        }
        *self.published.lock().unwrap() = Some((
            (*document.project).clone(),
            renders.iter().map(|render| render.source).collect(),
        ));
        Ok(())
    }
}

struct PausedHost {
    destination: Arc<HostDestination>,
    entered: Arc<std::sync::atomic::AtomicBool>,
    resume: Mutex<std::sync::mpsc::Receiver<()>>,
}

impl external::Destination for PausedHost {
    fn presentation(&self) -> external::Presentation {
        PRESENTATION
    }
    fn output(&self, document: &Document, target: OutputTarget) -> Result<output::Options, Error> {
        self.destination.output(document, target)
    }
    fn publish(&self, document: &Document, renders: &[external::Rendered]) -> Result<(), Error> {
        self.entered
            .store(true, std::sync::atomic::Ordering::Release);
        self.resume
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(15))
            .map_err(|_| Error::Project("host publication was not released".into()))?;
        self.destination.publish(document, renders)
    }
}

#[test]
fn external_completion_keeps_newer_accepted_edits_instead_of_closing_over_them() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    };
    struct Release(mpsc::Sender<()>);
    impl Drop for Release {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }

    let directory = tempfile::tempdir().unwrap();
    let destination = Arc::new(HostDestination {
        folder: directory.path().into(),
        published: Mutex::default(),
        reject: false,
    });
    let entered = Arc::new(AtomicBool::new(false));
    let (release, resume) = mpsc::channel();
    let release = Release(release);
    let request = external::Request::new(HostInput {
        paths: vec![fixture("patches.dng")],
        destination: Arc::new(PausedHost {
            destination: destination.clone(),
            entered: entered.clone(),
            resume: Mutex::new(resume),
        }),
    });
    editor_harness(|mut h, _| {
        h.enter(|editor, cx| editor.open_external(request, cx));
        wait_opened_photo(&mut h);
        h.update(|editor| {
            editor.set_dmin(Some(DVec3::splat(0.8)));
            editor.set_dmax(Some(DVec3::splat(0.02)));
            editor.document.project.roll.exposure = 0.25;
            let current = editor.document.visit.current_source.unwrap();
            editor.document.project.source_mut(current).unwrap().crop = Some(Crop {
                center: [120., 100.],
                size: [16, 12],
                angle: 0.,
            });
        });
        h.enter(|editor, cx| {
            editor.document.files.chooser_path = Some(directory.path().join("host.antitone"));
            editor.return_external(cx);
        });
        wait_for(&mut h, |_| entered.load(Ordering::Acquire));
        h.update(|editor| {
            assert!(editor.workspace_input.is_blocked());
            // Already-running work can accept a separate edit; a modal only reserves input.
            let mut edit = editor.edit_project("Later accepted edit").unwrap();
            edit.update(|project| project.roll.exposure = 0.75);
            editor.commit_project(edit).unwrap();
        });
        drop(release);
        wait_for(&mut h, |editor| !editor.document.external_edit.pending());
        h.read(|editor| {
            assert!(!editor.dialogs.is_open(), "{}", editor.status);
            assert!(
                !editor.allow_close,
                "returning a snapshot must not discard newer accepted work"
            );
            assert!(editor.document.dirty());
            assert_eq!(editor.document.project.roll.exposure, 0.75);
            assert!(
                !editor.external_return_available(),
                "this invocation already published its return"
            );
        });
        native_file_action(&mut h, crate::platform::menu::Action::Undo);
        wait_for(&mut h, |editor| !editor.interaction_blocked());
        h.read(|editor| {
            assert_eq!(editor.document.project.roll.exposure, 0.25);
            assert!(
                !editor.document.dirty(),
                "the saved baseline is the published snapshot"
            );
            assert!(
                !editor.allow_close,
                "completion does not replay after newer work ends"
            );
        });
        assert_eq!(
            destination
                .published
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .0
                .roll
                .exposure,
            0.25
        );
    });
    assert!(directory.path().join("patches.tif").is_file());
}

#[test]
fn external_edit_saves_the_project_before_rendering_and_publication() {
    for reject in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("host.antitone");
        let destination = Arc::new(HostDestination {
            folder: directory.path().into(),
            published: Mutex::default(),
            reject,
        });
        let request = external::Request::new(HostInput {
            paths: vec![fixture("patches.dng")],
            destination: destination.clone(),
        });
        editor_harness(|mut h, _| {
            h.enter(|editor, cx| editor.open_external(request, cx));
            wait_for(&mut h, |editor| !editor.interaction_blocked());
            h.update(|editor| {
                editor.set_dmin(Some(DVec3::splat(0.8)));
                editor.set_dmax(Some(DVec3::splat(0.02)));
                editor.document.project.roll.exposure = 0.5;
                let current = editor.document.visit.current_source.unwrap();
                editor.document.project.source_mut(current).unwrap().crop = Some(Crop {
                    center: [120., 100.],
                    size: [16, 12],
                    angle: 0.,
                });
                editor.document.files.chooser_path = Some(path.clone());
            });
            h.step();
            let button =
                h.get_by_role_and_label(egui::accesskit::Role::Button, "Return to Test Host");
            assert!(!button.accesskit_node().is_disabled());
            button.click();
            h.step();
            wait_for(&mut h, |editor| !editor.document.external_edit.pending());
            h.read(|editor| {
                assert_eq!(editor.document.path.as_ref(), Some(&path));
                assert_eq!(editor.allow_close, !reject);
                assert!(
                    !editor.document.dirty(),
                    "even a failed publication retains the completed project save"
                );
                assert_eq!(editor.dialogs.is_open(), reject);
            });
            let saved = Document::open(&path).unwrap();
            assert_eq!(saved.project.roll.exposure, 0.5);
            assert_eq!(saved.project.sources[0].crop.unwrap().size, [16, 12]);
            let published = destination.published.lock().unwrap();
            if reject {
                assert!(published.is_none());
            } else {
                let (project, returned) = published.as_ref().unwrap();
                assert_eq!(project.roll.exposure, 0.5);
                assert_eq!(*returned, project.order);
            }
        });
        assert!(directory.path().join("patches.tif").is_file());
    }
}

#[test]
fn external_session_survives_normal_import_and_history_then_retires_on_document_replacement() {
    external_session_import_history_and_departure(None, egui::vec2(1100., 800.), 315.);
}

#[test]
fn external_session_controls_history_and_discard_work_while_photo_loading_is_pending() {
    for (size, width) in [
        (egui::vec2(1100., 800.), 315.),
        (egui::vec2(800., 600.), 270.),
    ] {
        let held = settling::HeldPhoto::new();
        external_session_import_history_and_departure(Some(held), size, width);
    }
}

fn external_session_import_history_and_departure(
    mut held: Option<settling::HeldPhoto>,
    size: egui::Vec2,
    width: f32,
) {
    let directory = tempfile::tempdir().unwrap();
    let destination = Arc::new(HostDestination {
        folder: directory.path().into(),
        published: Mutex::default(),
        reject: false,
    });
    let weak = Arc::downgrade(&destination);
    let request = external::Request::new(HostInput {
        paths: vec![fixture("patches.dng")],
        destination,
    });
    editor_harness(|mut h, _| {
        set_layout(&mut h, size, width);
        if let Some(held) = &held {
            held.install(&h);
        }
        h.enter(|editor, cx| editor.open_external(request, cx));
        wait_for(&mut h, |editor| !editor.interaction_blocked());
        if let Some(held) = &held {
            held.wait_entered(&mut h);
        }
        let assert_loading = |editor: &Editor| {
            if let Some(held) = &held {
                assert!(held.sources.waiter_count() > 0);
                assert!(editor.document.visit.source.pending());
                assert!(editor.document.visit.canvas.ready.is_none());
                assert!(
                    editor
                        .activities
                        .any(|view| !view.modal && view.title == "Loading photo…")
                );
            }
        };
        h.update(|editor| {
            editor.set_dmin(Some(DVec3::splat(0.8)));
            editor.set_dmax(Some(DVec3::splat(0.02)));
        });
        h.step();
        let requested = h.read(|editor| {
            editor
                .document
                .external_edit
                .session
                .as_ref()
                .unwrap()
                .selected()
        });
        h.enter(|editor, cx| editor.add_raws(vec![fixture("roll-0.dng")], cx));
        wait_for(&mut h, |editor| !editor.interaction_blocked());
        h.read(|editor| {
            assert_eq!(editor.document.project.order.len(), 2);
            assert_eq!(
                editor
                    .document
                    .external_edit
                    .session
                    .as_ref()
                    .unwrap()
                    .selected(),
                requested
            );
        });
        h.read(assert_loading);
        stage(&mut h, "Roll");
        if h.query_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
            .is_none()
        {
            scroll_to_control(&mut h, egui::accesskit::Role::Button, "Exposure");
            h.get_by_role_and_label(egui::accesskit::Role::Button, "Exposure")
                .click();
            h.step();
        }
        workflow::enter_numeric(&mut h, "Exposure", "0.5");
        wait_for(&mut h, |editor| !editor.interaction_blocked());
        h.read(|editor| {
            assert_loading(editor);
            assert_eq!(editor.document.project.roll.exposure, 0.5);
        });
        native_file_action(&mut h, crate::platform::menu::Action::Undo);
        wait_for(&mut h, |editor| !editor.interaction_blocked());
        h.read(|editor| {
            assert_loading(editor);
            assert_eq!(editor.document.project.roll.exposure, 0.);
            assert_eq!(
                editor
                    .document
                    .external_edit
                    .session
                    .as_ref()
                    .unwrap()
                    .selected(),
                requested
            );
        });
        native_file_action(&mut h, crate::platform::menu::Action::Redo);
        wait_for(&mut h, |editor| !editor.interaction_blocked());
        h.read(|editor| {
            assert_loading(editor);
            assert_eq!(editor.document.project.roll.exposure, 0.5);
            assert_eq!(
                editor
                    .document
                    .external_edit
                    .session
                    .as_ref()
                    .unwrap()
                    .selected(),
                requested
            );
        });
        assert!(
            weak.upgrade().is_some(),
            "Redo must retain the caller before replacement retires it"
        );
        // Capture ordinary accepted data; it must not prolong caller ownership.
        let snapshot = h.read(|editor| editor.document.capture());
        h.enter(|editor, cx| editor.guard_transition(Transition::New, cx));
        wait_for(&mut h, |editor| editor.dialogs.is_open());
        h.read(assert_loading);
        h.get_by_label("Discard changes").click();
        h.step();
        wait_for(&mut h, |editor| {
            editor.document.project.order.is_empty() && !editor.document.files.pending()
        });
        h.read(|editor| assert!(editor.document.external_edit.session.is_none()));
        assert!(weak.upgrade().is_none());
        assert_eq!(snapshot.project.order.len(), 2);
        if let Some(held) = &mut held {
            assert_eq!(
                held.sources.waiter_count(),
                0,
                "departure must withdraw acquisition demand before loader shutdown"
            );
            held.release();
            held.sources.close_and_wait().unwrap();
            h.step();
            h.read(|editor| {
                assert!(editor.document.project.order.is_empty());
                assert!(editor.document.external_edit.session.is_none());
                assert!(!editor.document.visit.source.pending());
                assert!(editor.document.visit.canvas.ready.is_none());
                assert!(editor.canvas_frame.displayed.is_none());
            });
        }
    });
}
