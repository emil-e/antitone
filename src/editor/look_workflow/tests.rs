use super::*;
use crate::{
    application::test_support::with_application,
    editor::{
        FailureKind, Preparation, TaskRequest, document::Document, source, source_loader,
        tests::fixture,
    },
    looks::{LookOrigin, Lut3D, builtin::Builtin},
};
use std::{
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

fn asset(value: f32) -> Arc<LookAsset> {
    Arc::new(
        LookAsset::new(
            format!("Constant {value}"),
            LookOrigin::Imported,
            Arc::new(Lut3D::new(2, [0.; 3], [1.; 3], vec![[value; 3]; 8]).unwrap()),
        )
        .unwrap(),
    )
}

fn editor() -> (Editor, source_loader::SourceLoader) {
    let mut e = fixture();
    e.document = Document::default().into();
    e.set_dmin(None);
    e.set_dmax(None);
    e.set_floor(crate::processing::CAMERA_INPUT_FLOOR);
    e.sync_document();
    let (device, queue) = pollster::block_on(async {
        let instance = crate::gpu::instance();
        instance
            .request_adapter(&Default::default())
            .await
            .unwrap()
            .request_device(&Default::default())
            .await
            .unwrap()
    });
    let scheduler = source_loader::SourceLoader::new();
    let renderers = Arc::new(crate::renderer::RendererCache::new(e.submissions.clone()));
    e.source_services = Some(source::Services {
        developments: scheduler.clone(),
        renderers: renderers.clone(),
        device: device.clone(),
        queue: queue.clone(),
    });
    e.look_services = Some(Services::new(
        renderers,
        device,
        queue,
        e.submissions.clone(),
    ));
    (e, scheduler)
}

#[test]
fn captured_look_finishes_without_backend_or_paint_and_records_only_accepted_content() {
    let (e, scheduler) = editor();
    let selected = asset(0.4);
    with_application(e, |app| {
        let mut expected = app.read(|e| e.document.project.clone());
        app.enter(|e, access| {
            e.begin_look(Source::Loaded(selected.clone()), access);
        });
        app.run_until(Duration::from_secs(15), |e| e.document.look.is_none());
        expected.output.look = Some(selected.clone());
        app.read(|e| {
            assert_eq!(e.document.project, expected);
            assert_eq!(e.loaded_looks, vec![selected]);
            assert_eq!(
                e.history_label(crate::editor::history::Direction::Undo),
                Some("Creative look")
            );
            assert!(!e.busy && e.tasks.view().is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn import_and_loaded_reselection_need_no_original_file_and_failures_preserve_the_accepted_asset() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("look.cube");
    std::fs::write(
        &path,
        "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n",
    )
    .unwrap();
    let (e, scheduler) = editor();
    with_application(e, |app| {
        app.enter(|e, access| {
            e.begin_look(Source::Imported(path.clone()), access);
        });
        app.run_until(Duration::from_secs(15), |e| e.document.look.is_none());
        let accepted = app.read(|e| {
            e.document
                .project
                .output
                .look
                .clone()
                .expect("valid imported LUT")
        });
        let before = app.read(|e| e.document.project.clone());
        std::fs::write(&path, "LUT_1D_SIZE 2\n0 0 0\n1 1 1\n").unwrap();
        app.enter(|e, access| {
            e.begin_look(Source::Imported(path.clone()), access);
        });
        app.run_until(Duration::from_secs(15), |e| e.document.look.is_none());
        app.read(|e| {
            assert_eq!(e.document.project, before);
            assert_eq!(e.loaded_looks, vec![accepted.clone()]);
            assert_eq!(e.failure.as_ref().unwrap().kind, FailureKind::Look);
        });
        std::fs::remove_file(&path).unwrap();
        app.enter(|e, access| {
            e.begin_look(Source::Imported(path.clone()), access);
        });
        app.run_until(Duration::from_secs(15), |e| e.document.look.is_none());
        app.read(|e| {
            assert_eq!(e.failure.as_ref().unwrap().kind, FailureKind::Look);
            assert!(Arc::ptr_eq(
                e.document.project.output.look.as_ref().unwrap(),
                &accepted
            ));
        });
        app.enter(|e, access| {
            e.clear_look();
            e.begin_look(Source::Loaded(accepted.clone()), access);
        });
        app.run_until(Duration::from_secs(15), |e| e.document.look.is_none());
        app.read(|e| {
            assert_eq!(e.document.project.output.look.as_ref().unwrap(), &accepted);
            assert_eq!(e.loaded_looks.len(), 1);
            assert!(e.failure.is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn user_cancel_waits_for_acknowledgement_and_never_replaces_the_accepted_look() {
    let (mut e, scheduler) = editor();
    e.document.project.output.look = Some(asset(0.2));
    let before = e.document.project.clone();
    let hold = crate::background::testing::pause();
    with_application(e, |app| {
        let id = app.enter(|e, access| {
            e.begin_look(Source::Loaded(asset(0.7)), access);

            e.document.look.as_ref().unwrap().id
        });
        app.pump();
        app.update(|e| e.cancel_task(TaskRequest::Look(id)));
        app.read(|e| {
            assert!(
                e.busy
                    && e.document
                        .look
                        .as_ref()
                        .unwrap()
                        .cancel
                        .load(Ordering::Relaxed)
            );
            assert_eq!(e.document.project, before);
        });
        drop(hold);
        app.run_until(Duration::from_secs(15), |e| e.document.look.is_none());
        app.read(|e| {
            assert_eq!(e.document.project, before);
            assert!(!e.busy && e.tasks.view().is_none());
            assert!(e.loaded_looks.is_empty());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn superseded_look_drops_queued_input_but_the_current_operation_survives_photo_retirement() {
    let (e, scheduler) = editor();
    let hold = crate::background::testing::pause();
    let old = asset(0.3);
    let weak = Arc::downgrade(&old);
    let chosen = asset(0.7);
    with_application(e, |app| {
        let old_id = app.enter(|e, access| {
            e.begin_look(Source::Loaded(old), access);

            e.document.look.as_ref().unwrap().id
        });
        app.pump();
        app.enter(|e, access| {
            e.begin_look(Source::Loaded(chosen.clone()), access);

            let view = e.request_id();
            e.replace_photo_scope(view);
        });
        app.pump();
        assert!(weak.upgrade().is_none());
        app.update(|e| e.receive_look(old_id, Err("retired".into())));
        app.read(|e| assert!(e.busy && e.document.look.is_some()));
        drop(hold);
        app.run_until(Duration::from_secs(15), |e| e.document.look.is_none());
        app.read(|e| {
            assert_eq!(e.document.project.output.look, Some(chosen.clone()));
            assert_eq!(e.loaded_looks, vec![chosen]);
            assert!(e.failure.is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn document_transition_retires_look_but_keeps_session_comparison_assets() {
    let (mut e, scheduler) = editor();
    let previous = asset(0.2);
    e.document.project.output.look = Some(previous.clone());
    e.remember_current_look();
    let hold = crate::background::testing::pause();
    with_application(e, |app| {
        let old = app.enter(|e, access| {
            e.begin_look(Source::Loaded(asset(0.9)), access);

            e.document.look.as_ref().unwrap().id
        });
        app.pump();
        app.enter(|e, access| {
            e.start_document(Preparation::New, access);
        });
        let operation = app.read(|e| e.document_pending);
        app.update(|e| e.receive_look(old, Ok(Some(asset(0.9)))));
        app.read(|e| {
            assert_eq!(e.document_pending, operation);
            assert!(e.busy);
            assert_eq!(e.document.project.output.look, Some(previous.clone()));
        });
        drop(hold);
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert!(e.document.project.output.look.is_none());
            assert_eq!(e.loaded_looks, vec![previous]);
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn owner_and_application_retirement_cancel_queued_work_even_if_editor_data_is_still_alive() {
    for application in [false, true] {
        let (e, scheduler) = editor();
        let hold = crate::background::testing::pause();
        with_application(e, |app| {
            let (id, cancel) = app.enter(|e, access| {
                e.begin_look(Source::Loaded(asset(0.8)), access);
                let p = e.document.look.as_ref().unwrap();

                (p.id, p.cancel.clone())
            });
            app.pump();
            if application {
                app.close_execution();
            } else {
                app.update(|e| e.document = Document::default().into());
                app.pump();
            }
            assert!(cancel.load(Ordering::Relaxed));
            if !application {
                app.update(|e| e.receive_look(id, Ok(Some(asset(0.8)))));
            }
            app.read(|e| assert!(e.document.project.output.look.is_none()));
            drop(hold);
        });
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn retired_running_bake_keeps_inputs_without_blocking_an_independent_look() {
    let (e, scheduler) = editor();
    let cache = Arc::downgrade(&e.look_services.as_ref().unwrap().cache);
    let directory = e
        .look_services
        .as_ref()
        .unwrap()
        .cache
        ._directory
        .path()
        .to_owned();
    let (started, starts) = std::sync::mpsc::channel();
    let (release, resume) = std::sync::mpsc::channel();
    let resume = Mutex::new(resume);
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    e.submissions.observe_submissions(Arc::new(move || {
        if observed.fetch_add(1, Ordering::Relaxed) == 0 {
            started.send(()).unwrap();
            resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(20))
                .unwrap();
        }
    }));
    with_application(e, |app| {
        app.enter(|e, access| {
            e.begin_look(Source::Builtin(Builtin::Kodak2383), access);
        });
        app.pump();
        starts.recv_timeout(Duration::from_secs(15)).unwrap();
        let selected = asset(0.6);
        app.enter(|e, access| {
            e.retire_look();
            e.look_services = None;
            let source = e.source_services.as_ref().unwrap();
            e.look_services = Some(Services::new(
                source.renderers.clone(),
                source.device.clone(),
                source.queue.clone(),
                e.submissions.clone(),
            ));
            e.begin_look(Source::Loaded(selected.clone()), access);
        });
        app.run_until(Duration::from_secs(20), |e| e.document.look.is_none());
        assert!(cache.upgrade().is_some() && directory.exists());
        app.read(|e| assert_eq!(e.document.project.output.look, Some(selected.clone())));
        release.send(()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while cache.upgrade().is_some() || directory.exists() {
            app.pump();
            assert!(
                std::time::Instant::now() < deadline,
                "retired bake kept its inputs"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        app.read(|e| {
            assert_eq!(e.document.project.output.look, Some(selected));
            assert!(e.failure.is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn builtin_workflow_reuses_service_cache_across_documents_without_reinitializing_the_generator() {
    let (e, scheduler) = editor();
    with_application(e, |app| {
        app.enter(|e, access| {
            e.begin_look(Source::Builtin(Builtin::Kodak2383), access);
        });
        app.run_until(Duration::from_secs(30), |e| e.document.look.is_none());
        let accepted = app.read(|e| {
            let look = e
                .document
                .project
                .output
                .look
                .clone()
                .expect("cold builtin completed");
            assert_eq!(look.origin(), &Builtin::Kodak2383.origin());
            assert_eq!(look.lut().size(), crate::looks::builtin::DEFAULT_GRID);
            look
        });
        app.enter(|e, access| {
            e.start_document(Preparation::New, access);
        });
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        let fresh = app.update(|e| {
            assert!(e.document.project.output.look.is_none());
            let fresh = Arc::new(crate::renderer::RendererCache::new(e.submissions.clone()));
            e.look_services.as_mut().unwrap().renderers = fresh.clone();
            fresh
        });
        assert!(!fresh.is_initialized());
        app.enter(|e, access| {
            e.begin_look(Source::Builtin(Builtin::Kodak2383), access);
        });
        app.run_until(Duration::from_secs(30), |e| e.document.look.is_none());
        app.read(|e| assert_eq!(e.document.project.output.look, Some(accepted)));
        assert!(
            !fresh.is_initialized(),
            "the retained service cache must satisfy the exact builtin bake specification"
        );
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn unavailable_look_service_restores_idle_feedback_without_changing_the_project() {
    let mut e = fixture();
    e.sync_document();
    let before = e.document.project.clone();
    with_application(e, |app| {
        app.enter(|e, access| {
            e.begin_look(Source::Loaded(asset(0.4)), access);
            assert!(e.busy && e.document.look.as_ref().unwrap().task.is_some());
        });
        app.run_until(Duration::from_secs(5), |e| !e.busy);
        app.read(|e| {
            assert_eq!(e.document.project, before);
            assert!(!e.busy && e.tasks.view().is_none());
            assert_eq!(e.failure.as_ref().unwrap().kind, FailureKind::Look);
        });
    });
}
