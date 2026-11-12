use super::*;
use crate::application::test_support::{TestApplication, with_application};
use crate::project::{Project, Save, WritePhase};
use std::{
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

struct Release(Option<mpsc::Sender<()>>);
impl Release {
    fn now(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}
impl Drop for Release {
    fn drop(&mut self) {
        self.now();
    }
}

fn observe(
    document: &mut Document,
    at: WritePhase,
) -> (mpsc::Receiver<()>, mpsc::Receiver<()>, Release) {
    let (entered, starts) = mpsc::channel();
    let (published, completed) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Mutex::new(resume);
    document.write_observer = Some(Arc::new(move |phase| {
        if phase == at {
            entered.send(()).unwrap();
            resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(30))
                .unwrap();
        }
        if phase == WritePhase::Published {
            published.send(()).unwrap();
        }
    }));
    (starts, completed, Release(Some(release)))
}

fn wait_for(app: &mut TestApplication<'_, Editor>, entered: &mpsc::Receiver<()>) {
    let until = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        app.pump();
        if entered.try_recv().is_ok() {
            break;
        }
        assert!(
            std::time::Instant::now() < until,
            "write did not reach its observed boundary"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn retirement_at_real_publication_boundaries_preserves_the_file_but_cannot_accept_into_another_document()
 {
    for phase in [WritePhase::TemporarySynced, WritePhase::Published] {
        let mut editor = crate::editor::tests::fixture();
        editor.document.project.roll.exposure = 1.25;
        let expected = editor.document.project.clone();
        let (starts, published, mut release) = observe(&mut editor.document.state, phase);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("captured.antitone");
        with_application(editor, |app| {
            app.enter(|editor, cx| {
                editor.save_project(
                    path.clone(),
                    Save::New,
                    Some(super::super::Transition::Close),
                    cx,
                )
            });
            wait_for(app, &starts);
            assert_eq!(path.exists(), phase == WritePhase::Published);
            app.read(|editor| {
                assert!(editor.document.write.pending() && editor.interaction_blocked())
            });
            app.update(Editor::new_project);
            app.pump();
            release.now();
            published.recv_timeout(Duration::from_secs(15)).unwrap();
            app.pump();
            assert_eq!(Project::load(&path).unwrap(), expected);
            app.read(|editor| {
                assert!(editor.document.path.is_none() && editor.document.project.order.is_empty());
                assert_eq!(editor.document.project.roll.exposure, 0.);
                assert!(!editor.document.write.pending() && !editor.interaction_blocked());
                assert!(!editor.allow_close && !editor.dialogs.is_open());
            });
        });
    }
}

#[test]
fn file_workflow_retirement_does_not_disown_an_already_published_save() {
    let mut editor = crate::editor::tests::fixture();
    editor.document.project.roll.exposure = 0.75;
    let expected = editor.document.project.clone();
    let (starts, published, mut release) =
        observe(&mut editor.document.state, WritePhase::Published);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("published.antitone");
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.save_project(
                path.clone(),
                Save::New,
                Some(super::super::Transition::Close),
                cx,
            )
        });
        wait_for(app, &starts);
        let bytes = std::fs::read(&path).unwrap();
        app.update(|editor| editor.files = Default::default());
        app.pump();
        app.read(|editor| {
            assert!(editor.document.write.pending());
            assert!(
                editor.workspace_input.is_blocked(),
                "the write owns its own reservation"
            );
        });
        release.now();
        published.recv_timeout(Duration::from_secs(15)).unwrap();
        app.run_until(Duration::from_secs(15), |editor| {
            !editor.document.write.pending()
        });
        app.read(|editor| {
            assert!(!editor.document.dirty() && !editor.allow_close);
            assert_eq!(editor.document.project, expected);
            assert_eq!(editor.document.path.as_ref(), Some(&path));
        });
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    });
}

#[test]
fn selected_native_acquisition_can_finish_under_a_held_save_without_restarting_the_visit() {
    let mut editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    let id = editor.document.visit.current_source.unwrap();
    let source = editor.document.source_path(id).unwrap();
    let loader = crate::source::SourceLoader::new();
    editor.source_services.as_mut().unwrap().developments = loader.clone();
    editor.developments = Some(loader.clone());
    let (starts, published, mut release) =
        observe(&mut editor.document.state, WritePhase::TemporarySynced);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("during-load.antitone");
    with_application(editor, |app| {
        let (view, epoch) = app.enter(|editor, cx| {
            editor.load(source.clone(), cx);
            let view = editor.document.visit.view;
            let epoch = editor
                .source_epoch
                .load(std::sync::atomic::Ordering::Relaxed);
            editor.save_project(path.clone(), Save::New, None, cx);
            (view, epoch)
        });
        wait_for(app, &starts);
        app.run_until(Duration::from_secs(15), |editor| {
            !editor.document.visit.source.pending()
        });
        app.read(|editor| {
            assert!(
                editor.document.visit.canvas.ready.is_some(),
                "actual native acquisition must complete before releasing the write"
            );
            assert!(editor.document.write.pending() && editor.interaction_blocked());
            assert!(editor.workspace_input.is_blocked());
            assert_eq!(editor.document.visit.view, view);
            assert_eq!(
                editor
                    .source_epoch
                    .load(std::sync::atomic::Ordering::Relaxed),
                epoch
            );
        });
        release.now();
        published.recv_timeout(Duration::from_secs(15)).unwrap();
        app.run_until(Duration::from_secs(15), |editor| {
            !editor.document.write.pending()
        });
        app.read(|editor| {
            assert_eq!(editor.document.visit.view, view);
            assert!(!editor.document.dirty());
        });
    });
    loader.close_and_wait().unwrap();
}

#[test]
fn a_real_devices_lut_capacity_does_not_limit_saved_json() {
    use crate::looks::{LookAsset, LookOrigin, Lut3D};
    let instance = crate::gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: wgpu::Limits {
            max_storage_buffer_binding_size: 1024,
            ..Default::default()
        },
        ..Default::default()
    }))
    .unwrap();
    let asset = Arc::new(
        LookAsset::new(
            "Stored samples".into(),
            LookOrigin::Imported,
            Arc::new(Lut3D::new(5, [0.; 3], [1.; 3], vec![[0.5; 3]; 125]).unwrap()),
        )
        .unwrap(),
    );
    assert!(125 * 16 > device.limits().max_storage_buffer_binding_size);
    assert!(
        crate::gpu::lut3d::Lut::new(&device, asset.lut()).is_err(),
        "the consuming device must actually reject this resource"
    );
    let mut editor = crate::editor::tests::fixture();
    editor.document.project.output.lut = Some(asset);
    let loader = crate::source::SourceLoader::new();
    loader.close_and_wait().unwrap();
    editor.source_services = Some(super::super::source::Services {
        developments: loader,
        renderers: Arc::default(),
        device,
        queue,
    });
    let expected = editor.document.project.clone();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("portable.antitone");
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.save_project(path.clone(), Save::New, None, cx));
        app.run_until(Duration::from_secs(15), |editor| {
            !editor.document.write.pending()
        });
        app.read(|editor| assert!(!editor.document.dirty() && !editor.dialogs.is_open()));
        assert_eq!(Document::open(&path).unwrap().project, expected);
        assert_eq!(Project::load(&path).unwrap(), expected);
    });
}

#[test]
fn an_after_save_close_reprompts_for_accepted_edits_that_were_not_in_the_written_snapshot() {
    let mut editor = crate::editor::tests::fixture();
    editor.document.project.roll.exposure = 0.25;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("older-snapshot.antitone");
    let hold = crate::background::testing::pause();
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.save_project(
                path.clone(),
                Save::New,
                Some(super::super::Transition::Close),
                cx,
            );
            let mut edit = editor.edit_project("Later accepted exposure").unwrap();
            edit.update(|project| project.roll.exposure = 0.75);
            editor.commit_project(edit).unwrap();
        });
        app.pump();
        drop(hold);
        app.run_until(Duration::from_secs(15), |editor| {
            !editor.document.write.pending() && editor.dialogs.is_open()
        });
        assert_eq!(Project::load(&path).unwrap().roll.exposure, 0.25);
        app.update(|editor| {
            assert!(!editor.allow_close && editor.document.dirty());
            assert_eq!(editor.document.project.roll.exposure, 0.75);
            assert_eq!(
                editor.history_label(super::super::history::Direction::Undo),
                Some("Later accepted exposure")
            );
            editor.dialogs.answer_for_test("Cancel");
        });
        app.pump();
        app.read(|editor| {
            assert!(!editor.files.pending() && !editor.allow_close && editor.document.dirty())
        });
    });
}

#[test]
fn a_conflicting_edit_declines_save_without_retiring_the_edit_or_queueing_a_transition() {
    let mut editor = crate::editor::tests::fixture();
    editor.document.project.roll.exposure = 0.75;
    let mut edit = editor.edit_project("Unfinished controls").unwrap();
    edit.update(|project| project.roll.exposure = 1.5);
    let preview = edit.preview();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("accepted.antitone");
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.save_project(
                path.clone(),
                Save::New,
                Some(super::super::Transition::Close),
                cx,
            )
        });
        app.pump();
        app.read(|editor| {
            assert!(preview.active());
            assert_eq!(editor.document.project.roll.exposure, 0.75);
            assert!(!editor.document.write.pending() && !editor.files.pending());
            assert!(!editor.allow_close && !editor.dialogs.is_open());
        });
        assert!(!path.exists());
        drop(edit);
        app.enter(|editor, cx| editor.save_project(path.clone(), Save::New, None, cx));
        app.run_until(Duration::from_secs(15), |editor| {
            !editor.document.write.pending()
        });
        assert_eq!(Project::load(&path).unwrap().roll.exposure, 0.75);
    });
}

#[test]
fn metadata_save_and_open_preserve_missing_or_corrupt_photos_and_look_data_without_services() {
    use crate::looks::{LookAsset, LookOrigin, Lut3D};
    for corrupt in [false, true] {
        let raw = crate::raw::fixture::bayer_dng_with_as_shot([1800, 2200, 2800], [2, 1, 3]);
        let mut state = Document::default();
        state.import(&[raw.path().into()]).unwrap();
        if corrupt {
            std::fs::write(raw.path(), b"unavailable RAW content").unwrap();
        } else {
            std::fs::remove_file(raw.path()).unwrap();
        }
        state.project.output.lut = Some(Arc::new(
            LookAsset::new(
                "Captured look".into(),
                LookOrigin::Imported,
                Arc::new(Lut3D::new(5, [0.; 3], [1.; 3], vec![[0.5; 3]; 125]).unwrap()),
            )
            .unwrap(),
        ));
        let expected = state.project.clone();
        let mut editor = crate::editor::tests::fixture();
        editor.document = state.into();
        assert!(editor.source_services.is_none() && editor.developments.is_none());
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("metadata.antitone");
        with_application(editor, |app| {
            app.enter(|editor, cx| editor.save_project(path.clone(), Save::New, None, cx));
            app.run_until(Duration::from_secs(15), |editor| {
                !editor.document.write.pending()
            });
            app.read(|editor| {
                assert!(!editor.document.dirty() && !editor.dialogs.is_open());
                assert_eq!(editor.document.project, expected);
            });
            assert_eq!(Document::open(&path).unwrap().project, expected);
            assert_eq!(Project::load(&path).unwrap(), expected);
        });
    }
}
