use super::*;
use std::sync::Mutex;

pub(super) struct HeldPhoto {
    pub(super) sources: source_loading::SourceLoader,
    entered: mpsc::Receiver<()>,
    release: Option<mpsc::Sender<()>>,
}

impl HeldPhoto {
    pub(super) fn new() -> Self {
        let (started, entered) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let resume = Mutex::new(Some(resume));
        let sources = source_loading::SourceLoader::new_observed(move |_, phase, _| {
            if matches!(phase, source_loading::NativePhase::Open)
                && let Some(resume) = resume.lock().unwrap().take()
            {
                started.send(()).unwrap();
                resume.recv_timeout(Duration::from_secs(60)).unwrap();
            }
            Ok(())
        });
        Self {
            sources,
            entered,
            release: Some(release),
        }
    }

    pub(super) fn install(&self, harness: &Harness<'_, Editor>) {
        harness.update(|editor| {
            editor.source_services.as_mut().unwrap().developments = self.sources.clone();
        });
    }

    pub(super) fn wait_entered(&self, harness: &mut Harness<'_, Editor>) {
        wait_for(harness, |_| self.entered.try_recv().is_ok());
        harness.read(|editor| {
            assert!(editor.document.visit.source.pending());
            assert!(editor.document.visit.canvas.ready.is_none());
        });
    }

    pub(super) fn release(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
    }
}

impl Drop for HeldPhoto {
    fn drop(&mut self) {
        self.release();
    }
}

fn saved_roll(directory: &std::path::Path) -> (PathBuf, crate::project::Project) {
    let mut project = crate::project::Project::default();
    for name in ["roll-0.dng", "roll-1.dng"] {
        project
            .add_source(document::read_source(&fixture(name), Some([1.; 4])).unwrap())
            .unwrap();
    }
    let path = directory.join("pending-photos.antitone");
    project.save(&path, crate::project::Save::New).unwrap();
    (path, project)
}

#[test]
fn open_fixture_returns_accepted_metadata_before_selected_photo_finishes() {
    let directory = tempfile::tempdir().unwrap();
    let (path, project) = saved_roll(directory.path());
    let held = HeldPhoto::new();
    let sources = held.sources.clone();
    editor_harness(|mut harness, _| {
        let mut held = held;
        held.install(&harness);
        open_project(&mut harness, path);
        held.wait_entered(&mut harness);
        harness.read(|editor| {
            assert!(!editor.files.pending() && !editor.interaction_blocked());
            assert_eq!(editor.document.project, project);
            assert!(!editor.document.dirty());
        });
        held.release();
        wait_opened_photo(&mut harness);
        harness.read(|editor| {
            assert!(
                editor.document.visit.canvas.ready.is_some(),
                "{}",
                editor.status
            )
        });
    });
    sources.close_and_wait().unwrap();
}

#[test]
fn analyze_shelf_click_starts_its_own_preparation_before_selected_photo_finishes() {
    let directory = tempfile::tempdir().unwrap();
    let (path, _) = saved_roll(directory.path());
    let held = HeldPhoto::new();
    let sources = held.sources.clone();
    editor_harness(|mut harness, _| {
        let mut held = held;
        held.install(&harness);
        open_project(&mut harness, path);
        held.wait_entered(&mut harness);
        analyze::click_analyze(&mut harness);
        harness.read(|editor| {
            assert!(
                editor.document.analysis_operation().is_some(),
                "{}",
                editor.status
            );
            assert!(editor.document.visit.source.pending());
        });
        held.release();
        wait_for(&mut harness, |editor| {
            editor.document.analysis_review().is_some()
                || editor.document.analysis_operation().is_none()
        });
        harness.read(|editor| {
            assert!(
                editor.document.analysis_review().is_some(),
                "{}",
                editor.status
            )
        });
        wait_opened_photo(&mut harness);
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Cancel")
            .click();
        harness.step();
        wait_for(&mut harness, |editor| {
            editor.document.analysis_operation().is_none()
        });
    });
    sources.close_and_wait().unwrap();
}

#[test]
fn open_fixture_presents_an_actionable_decision_during_independent_photo_loading() {
    let directory = tempfile::tempdir().unwrap();
    let (path, _) = saved_roll(directory.path());
    let held = HeldPhoto::new();
    let sources = held.sources.clone();
    editor_harness(|mut harness, _| {
        let mut held = held;
        held.install(&harness);
        open_project(&mut harness, path.clone());
        held.wait_entered(&mut harness);
        harness.enter(|editor, cx| {
            editor.roll_control(
                ui::ControlResponse::action(egui::Id::new("dirty-open"), true),
                cx,
                |project| project.roll.exposure = 0.25,
            );
        });
        let before = harness.read(|editor| {
            assert!(editor.document.dirty());
            editor.document.project.clone()
        });
        for size in [egui::vec2(1100., 800.), egui::vec2(800., 600.)] {
            harness.set_size(size);
            open_project(&mut harness, path.clone());
            wait_for(&mut harness, |editor| editor.dialogs.is_open());
            harness.read(|editor| {
                assert!(editor.files.pending() && editor.dialogs.is_open());
                assert!(editor.document.visit.source.pending());
            });
            let cancel = harness.get_by_role_and_label(egui::accesskit::Role::Button, "Cancel");
            assert!(!cancel.accesskit_node().is_disabled());
            assert!(harness.ctx.content_rect().contains_rect(cancel.rect()));
            cancel.click();
            harness.step();
            wait_for(&mut harness, |editor| !editor.files.pending());
            harness.read(|editor| {
                assert!(!editor.dialogs.is_open());
                assert_eq!(editor.document.project, before);
                assert!(editor.document.visit.source.pending());
            });
        }
        held.release();
        wait_opened_photo(&mut harness);
    });
    sources.close_and_wait().unwrap();
}
