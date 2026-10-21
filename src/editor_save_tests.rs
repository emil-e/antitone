use super::*;
use crate::{
    application::test_support::with_application,
    editor::{
        Preparation, document_services, source,
        tests::{fixture, hold_blocking_work},
    },
};
use std::{path::PathBuf, sync::Arc, time::Duration};

fn gpu() -> (wgpu::Device, wgpu::Queue) {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        instance
            .request_adapter(&Default::default())
            .await
            .unwrap()
            .request_device(&Default::default())
            .await
            .unwrap()
    })
}

fn editor() -> (Editor, crate::editor::development::Scheduler) {
    let mut editor = fixture();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng");
    let mut document = Document::default();
    document.import(&[path]).unwrap();
    editor.document.install(document);
    editor.current_source = editor.document.project.order.first().copied();
    let (device, queue) = gpu();
    let scheduler = crate::editor::development::Scheduler::new();
    editor.source_services = Some(source::Services {
        developments: scheduler.clone(),
        jobs: editor.workflow_jobs.clone(),
        renderers: Arc::default(),
        device,
        queue,
    });
    (editor, scheduler)
}

#[test]
fn live_save_without_backend_or_paint_preserves_later_edits_history_visit_and_source_locators() {
    let (editor, scheduler) = editor();
    let hold = hold_blocking_work(&editor.workflow_jobs);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("written.antitone");
    with_application(editor, |app| {
        let (visit, store, locator) = app.read(|e| {
            (
                e.document.visit.view,
                e.document.cache().clone(),
                e.document.source_path(e.current_source.unwrap()).unwrap(),
            )
        });
        app.enter(|e, access| {
            e.roll.exposure = 0.25;
            e.save_project(path.clone(), crate::project::Save::New, None, access);
        });
        app.pump();
        assert!(app.read(|e| e.busy && e.document_pending.is_some()));
        app.update(|e| {
            let before = e.history_snapshot();
            e.roll.exposure = 0.5;
            e.record_edit(before, "Later exposure", None);
        });
        drop(hold);
        app.run_until(Duration::from_secs(15), |e| e.document_pending.is_none());
        assert_eq!(
            crate::project::Project::load(&path).unwrap().roll.exposure,
            0.25
        );
        app.read(|e| {
            assert_eq!(e.document.project.roll.exposure, 0.5);
            assert!(e.document.dirty());
            assert_eq!(
                e.history_label(crate::editor::history::Direction::Undo),
                Some("Later exposure")
            );
            assert_eq!(e.document.visit.view, visit);
            assert!(Arc::ptr_eq(e.document.cache(), &store) && !store.is_closed());
            assert_eq!(
                e.document.source_path(e.current_source.unwrap()).unwrap(),
                locator
            );
            assert!(!e.busy && e.tasks.view().is_none() && e.failure.is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn failed_live_save_preserves_destination_dirty_state_and_guarded_transition() {
    let (editor, scheduler) = editor();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("occupied.antitone");
    std::fs::write(&path, b"existing destination").unwrap();
    with_application(editor, |app| {
        app.enter(|e, access| {
            e.save_project(
                path.clone(),
                crate::project::Save::New,
                Some(crate::editor::Transition::Close),
                access,
            );
        });
        let accepted = app.read(|e| e.document.project.clone());
        app.run_until(Duration::from_secs(15), |e| e.document_pending.is_none());
        assert_eq!(std::fs::read(&path).unwrap(), b"existing destination");
        app.read(|e| {
            assert_eq!(e.document.project, accepted);
            assert!(e.document.dirty());
            assert_eq!(e.transition, Some(crate::editor::Transition::Close));
            assert!(e.document_pending.is_none() && !e.allow_close);
            assert_eq!(e.failure.as_ref().unwrap().kind, FailureKind::Save);
            assert!(!e.busy && e.tasks.view().is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn retiring_queued_write_drops_its_continuation_and_cannot_finish_a_new_operation() {
    let (editor, scheduler) = editor();
    let hold = hold_blocking_work(&editor.workflow_jobs);
    let jobs = editor.workflow_jobs.clone();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("retired.antitone");
    with_application(editor, |app| {
        app.enter(|e, access| {
            e.save_project(path.clone(), crate::project::Save::New, None, access);
        });
        let retired = app.read(|e| e.document_pending.unwrap());
        app.pump();
        app.enter(|e, cx| e.start_document(Preparation::New, cx));
        let current = app.read(|e| e.document_pending.unwrap());
        app.pump();
        drop(hold);
        pollster::block_on(jobs.run(|| {})).unwrap();
        app.enter(|e, cx| e.save_result(retired, Err("stale write failure".into()), cx));
        app.read(|e| {
            assert_eq!(e.document_pending, Some(current));
            assert!(e.busy && e.tasks.view().is_some());
            assert!(e.failure.is_none());
        });
        assert!(!path.exists());
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn cancelling_the_after_save_decision_does_not_cancel_the_accepted_write_or_close() {
    let (editor, scheduler) = editor();
    let hold = hold_blocking_work(&editor.workflow_jobs);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("saved.antitone");
    with_application(editor, |app| {
        app.enter(|e, access| {
            e.save_project(
                path.clone(),
                crate::project::Save::New,
                Some(crate::editor::Transition::Close),
                access,
            );
        });
        app.pump();
        app.update(|e| e.after_save = None);
        drop(hold);
        app.run_until(Duration::from_secs(15), |e| e.document_pending.is_none());
        assert!(path.exists());
        app.read(|e| {
            assert!(!e.document.dirty());
            assert!(e.after_save.is_none() && e.transition.is_none() && !e.allow_close);
            assert!(e.document_pending.is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn successful_save_continues_new_open_and_close_without_another_ui_pass() {
    use crate::editor::Transition;
    let directory = tempfile::tempdir().unwrap();
    let opened = directory.path().join("opened.antitone");
    crate::project::Project::default()
        .save(&opened, crate::project::Save::New)
        .unwrap();
    for (name, transition) in [
        ("new", Transition::New),
        ("open", Transition::Open(opened.clone())),
        ("close", Transition::Close),
    ] {
        let (editor, scheduler) = editor();
        let written = directory.path().join(format!("written-{name}.antitone"));
        let store = editor.document.cache().clone();
        with_application(editor, |app| {
            let captured = app.enter(|e, access| {
                e.save_project(
                    written.clone(),
                    crate::project::Save::New,
                    Some(transition.clone()),
                    access,
                );

                e.document.project.clone()
            });
            app.run_until(Duration::from_secs(15), |e| {
                !e.busy && e.document_pending.is_none()
            });
            assert_eq!(crate::project::Project::load(&written).unwrap(), captured);
            app.read(|e| {
                assert!(e.failure.is_none(), "{}", e.status);
                assert!(
                    e.after_save.is_none() && e.transition.is_none() && e.tasks.view().is_none()
                );
                match &transition {
                    Transition::New => {
                        assert!(e.document.project.order.is_empty() && e.document.path.is_none());
                        assert!(store.is_closed() && !e.allow_close);
                    }
                    Transition::Open(path) => {
                        assert_eq!(e.document.path.as_ref(), Some(path));
                        assert!(
                            e.document.project.order.is_empty()
                                && store.is_closed()
                                && !e.allow_close
                        );
                    }
                    Transition::Close => {
                        assert_eq!(
                            e.application_quit_state(),
                            crate::platform::host::QuitState::Approved
                        );
                        assert_eq!(e.document.path.as_ref(), Some(&written));
                        assert_eq!(e.document.project, captured);
                        assert!(!e.document.dirty() && !store.is_closed());
                        assert!(!e.file_available());
                    }
                }
            });
        });
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn failed_open_after_successful_save_keeps_the_written_document_without_a_ui_pass() {
    let (editor, scheduler) = editor();
    let directory = tempfile::tempdir().unwrap();
    let written = directory.path().join("written.antitone");
    let missing = directory.path().join("missing.antitone");
    with_application(editor, |app| {
        let accepted = app.enter(|e, access| {
            e.save_project(
                written.clone(),
                crate::project::Save::New,
                Some(crate::editor::Transition::Open(missing)),
                access,
            );

            e.document.project.clone()
        });
        app.run_until(Duration::from_secs(15), |e| {
            !e.busy && e.document_pending.is_none()
        });
        app.read(|e| {
            assert_eq!(e.document.path.as_ref(), Some(&written));
            assert_eq!(e.document.project, accepted);
            assert!(!e.document.dirty() && !e.allow_close);
            assert!(e.status.starts_with("Project: "), "{}", e.status);
        });
        assert_eq!(crate::project::Project::load(&written).unwrap(), accepted);
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn candidate_services_validate_authored_inputs_without_editor_and_leave_parent_open_on_failure() {
    let (device, _queue) = gpu();
    let mut document = Document::default();
    document
        .import(&[PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng")])
        .unwrap();
    let parent = document.cache().clone();
    let accepted = document.project.clone();
    let source = document.project.order[0];
    let prepared = document_services::candidate(
        crate::editor::DocumentRequest {
            work: Preparation::Correction(document.clone(), Some(source)),
            source: None,
            correction: None,
        },
        &device,
        |_, _| {},
    )
    .unwrap();
    assert_eq!(prepared.prepared.document.project, accepted);
    assert!(prepared.source.is_some());
    assert!(Arc::ptr_eq(
        prepared.prepared.document.parent_cache().unwrap(),
        &parent
    ));
    drop(prepared);
    assert!(!parent.is_closed());
    let bad = document_services::candidate(
        crate::editor::DocumentRequest {
            work: Preparation::Import(
                document.clone(),
                vec![PathBuf::from("missing-source.dng")],
                None,
            ),
            source: None,
            correction: None,
        },
        &device,
        |_, _| {},
    );
    assert!(bad.is_err());
    assert_eq!(document.project, accepted);
    assert!(!parent.is_closed());
}
