use super::*;
use crate::{
    application::test_support::with_application,
    editor::{FailureKind, Preparation, TaskRequest, document::Document, tests::fixture},
    looks::{Lut3D, builtin::Builtin},
};
use std::{
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use crate::editor::{source, source_loading};

pub(super) fn asset(value: f32) -> Arc<LookAsset> {
    Arc::new(
        LookAsset::new(
            format!("Constant {value}"),
            LookOrigin::Imported,
            Arc::new(Lut3D::new(2, [0.; 3], [1.; 3], vec![[value; 3]; 8]).unwrap()),
        )
        .unwrap(),
    )
}

pub(super) fn editor() -> (Editor, source_loading::SourceLoader) {
    editor_with_limits(Default::default())
}

pub(super) fn editor_with_limits(limits: wgpu::Limits) -> (Editor, source_loading::SourceLoader) {
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
            .request_device(&wgpu::DeviceDescriptor {
                required_limits: limits,
                ..Default::default()
            })
            .await
            .unwrap()
    });
    let scheduler = source_loading::SourceLoader::new();
    let renderers = Arc::new(crate::renderer::RendererCache::new(e.submissions.clone()));
    e.source_services = Some(source::Services {
        developments: scheduler.clone(),
        renderers: renderers.clone(),
        device: device.clone(),
        queue: queue.clone(),
    });
    e.look.services = Some(Services::new(device, queue, e.submissions.clone()));
    (e, scheduler)
}

fn assert_loaded_assets(editor: &Editor, expected: &[Arc<LookAsset>]) {
    assert_eq!(editor.look.loaded.len(), expected.len());
    for asset in expected {
        assert!(editor.look.loaded.contains(asset));
    }
}

#[test]
fn captured_choice_accepts_without_paint_and_records_only_accepted_content() {
    let (e, scheduler) = editor();
    let selected = asset(0.4);
    with_application(e, |app| {
        let mut expected = app.read(|e| e.document.state.project.clone());
        app.enter(|e, access| {
            e.select_look(Source::Loaded(selected.clone()), access);
        });
        app.run_until(Duration::from_secs(15), |e| e.document.look.is_none());
        expected.output.look = Some(selected.clone());
        app.read(|e| {
            assert_eq!(e.document.state.project, expected);
            assert_eq!(e.look.loaded, vec![selected]);
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
fn awaited_choices_keep_a_unique_session_list_and_one_undoable_accepted_selection() {
    let (e, scheduler) = editor();
    let first = asset(0.2);
    let second = asset(0.7);
    let renamed = Arc::new(
        LookAsset::new(
            "Renamed first".into(),
            crate::looks::LookOrigin::Imported,
            first.lut().clone(),
        )
        .unwrap(),
    );
    with_application(e, |app| {
        for chosen in [&first, &second, &renamed] {
            app.enter(|e, cx| e.select_look(Source::Loaded(chosen.clone()), cx));
            app.run_until(Duration::from_secs(15), |e| e.document.look.is_none());
            app.read(|e| assert_eq!(e.document.state.project.output.look.as_ref(), Some(chosen)));
        }
        let list = vec![renamed.clone(), second];
        app.read(|e| assert_loaded_assets(e, &list));
        app.update(Editor::clear_look);
        app.read(|e| {
            assert!(e.document.state.project.output.look.is_none());
            assert_loaded_assets(e, &list);
        });
        app.enter(|e, cx| e.restore_history(crate::editor::history::Direction::Undo, cx));
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert_eq!(e.document.state.project.output.look, Some(renamed));
            assert_loaded_assets(e, &list);
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn cancelling_a_prepared_value_retires_its_continuation_instead_of_accepting_it() {
    for cancelled in [false, true] {
        let (mut e, scheduler) = editor();
        let accepted = asset(0.2);
        let selected = asset(0.7);
        e.document.state.project.output.look = Some(accepted.clone());
        e.remember_imported_look();
        let before = e.document.state.project.clone();
        let (prepared, ready) = std::sync::mpsc::channel();
        let (release, resume) = std::sync::mpsc::channel();
        let (finished, done) = std::sync::mpsc::channel();
        let resume = Mutex::new(resume);
        e.look.services.as_mut().unwrap().prepared = Some(Arc::new(move |valid, stop| {
            prepared.send((valid, stop)).unwrap();
            resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(15))
                .unwrap();
            finished.send(()).unwrap();
        }));
        with_application(e, |app| {
            let activity = app.enter(|e, cx| {
                e.select_look(Source::Loaded(selected.clone()), cx);
                e.document.look.as_ref().unwrap().activity
            });
            app.pump();
            let (valid, stop) = ready.recv_timeout(Duration::from_secs(15)).unwrap();
            assert!(valid && !stop.is_cancelled());
            if cancelled {
                app.update(|e| e.cancel_task(TaskRequest::Look(activity)));
            }
            app.read(|e| {
                assert_eq!(e.document.look.is_none(), cancelled);
                assert_eq!(e.busy, !cancelled);
                assert_eq!(e.document.state.project, before);
            });
            if cancelled {
                app.pump();
                assert!(stop.is_cancelled());
            }
            release.send(()).unwrap();
            done.recv_timeout(Duration::from_secs(15)).unwrap();
            app.run_until(Duration::from_secs(15), |e| e.document.look.is_none());
            app.read(|e| {
                assert!(!e.busy && e.tasks.view().is_none());
                if cancelled {
                    assert_eq!(e.document.state.project, before);
                    assert_eq!(e.look.loaded, vec![accepted]);
                    assert!(
                        e.history_label(crate::editor::history::Direction::Undo)
                            .is_none()
                    );
                } else {
                    assert_eq!(e.document.state.project.output.look, Some(selected.clone()));
                    assert_loaded_assets(e, &[accepted, selected]);
                    assert_eq!(
                        e.history_label(crate::editor::history::Direction::Undo),
                        Some("Creative look")
                    );
                }
            });
        });
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn device_preflight_rejects_an_unsupported_asset_without_remembering_or_recording_it() {
    let (mut e, scheduler) = editor_with_limits(wgpu::Limits {
        max_storage_buffer_binding_size: 1024,
        ..Default::default()
    });
    assert_eq!(
        e.source_services
            .as_ref()
            .unwrap()
            .device
            .limits()
            .max_storage_buffer_binding_size,
        1024
    );
    let accepted = asset(0.2);
    e.document.state.project.output.look = Some(accepted.clone());
    e.remember_imported_look();
    let before = e.document.state.project.clone();
    let unsupported = Arc::new(
        LookAsset::new(
            "Too large for this device".into(),
            crate::looks::LookOrigin::Imported,
            Arc::new(crate::looks::Lut3D::new(5, [0.; 3], [1.; 3], vec![[0.5; 3]; 125]).unwrap()),
        )
        .unwrap(),
    );
    with_application(e, |app| {
        app.enter(|e, cx| e.select_look(Source::Loaded(unsupported), cx));
        app.run_until(Duration::from_secs(15), |e| e.document.look.is_none());
        app.read(|e| {
            assert_eq!(e.document.state.project, before);
            assert_eq!(e.look.loaded, vec![accepted.clone()]);
            assert!(
                e.history_label(crate::editor::history::Direction::Undo)
                    .is_none()
            );
            assert_eq!(e.failure.as_ref().unwrap().kind, FailureKind::Look);
        });
        // Acknowledge the failure at the feedback boundary before another choice.
        app.update(|e| e.resolve_failure(FailureKind::Look));
        let supported = asset(0.7);
        app.enter(|e, cx| e.select_look(Source::Loaded(supported.clone()), cx));
        app.run_until(Duration::from_secs(15), |e| e.document.look.is_none());
        app.read(|e| {
            assert_eq!(e.document.state.project.output.look, Some(supported));
            assert!(e.failure.is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn running_builtin_cancel_retires_the_action_while_its_gpu_unit_finishes() {
    let (mut e, scheduler) = editor();
    e.document.state.project.output.look = Some(asset(0.2));
    let before = e.document.state.project.clone();
    let (entered, started) = std::sync::mpsc::channel();
    let (release, resume) = std::sync::mpsc::channel();
    let (completed, done) = std::sync::mpsc::channel();
    let resume = Mutex::new(resume);
    let once = AtomicUsize::new(0);
    e.submissions.observe_submissions(Arc::new(move || {
        if once.fetch_add(1, Ordering::Relaxed) == 0 {
            entered.send(()).unwrap();
            resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(20))
                .unwrap();
        }
    }));
    e.look.services.as_mut().unwrap().prepared = Some(Arc::new(move |valid, stop| {
        completed.send((valid, stop.is_cancelled())).unwrap();
    }));
    with_application(e, |app| {
        let activity = app.enter(|e, cx| {
            e.select_look(Source::Builtin(Builtin::Kodak2383), cx);
            e.document.look.as_ref().unwrap().activity
        });
        app.pump();
        started.recv_timeout(Duration::from_secs(15)).unwrap();
        app.update(|e| e.cancel_task(TaskRequest::Look(activity)));
        app.read(|e| {
            assert!(!e.busy && e.document.look.is_none() && e.tasks.view().is_none());
            assert_eq!(e.document.state.project, before);
        });
        app.pump();
        release.send(()).unwrap();
        assert_eq!(
            done.recv_timeout(Duration::from_secs(20)).unwrap(),
            (false, true)
        );
        app.pump();
        app.read(|e| {
            assert_eq!(e.document.state.project, before);
            assert!(
                e.history_label(crate::editor::history::Direction::Undo)
                    .is_none()
            );
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
            e.select_look(Source::Imported(path.clone()), access);
        });
        app.run_until(Duration::from_secs(15), |e| e.document.look.is_none());
        let accepted = app.read(|e| {
            e.document
                .state
                .project
                .output
                .look
                .clone()
                .expect("valid imported LUT")
        });
        let before = app.read(|e| e.document.state.project.clone());
        std::fs::write(&path, "LUT_1D_SIZE 2\n0 0 0\n1 1 1\n").unwrap();
        app.enter(|e, access| {
            e.select_look(Source::Imported(path.clone()), access);
        });
        app.run_until(Duration::from_secs(15), |e| e.document.look.is_none());
        app.read(|e| {
            assert_eq!(e.document.state.project, before);
            assert_eq!(e.look.loaded, vec![accepted.clone()]);
            assert_eq!(e.failure.as_ref().unwrap().kind, FailureKind::Look);
        });
        std::fs::remove_file(&path).unwrap();
        app.enter(|e, access| {
            e.select_look(Source::Imported(path.clone()), access);
        });
        app.run_until(Duration::from_secs(15), |e| e.document.look.is_none());
        app.read(|e| {
            assert_eq!(e.failure.as_ref().unwrap().kind, FailureKind::Look);
            assert!(Arc::ptr_eq(
                e.document.state.project.output.look.as_ref().unwrap(),
                &accepted
            ));
        });
        app.enter(|e, access| {
            e.clear_look();
            e.select_look(Source::Loaded(accepted.clone()), access);
        });
        app.run_until(Duration::from_secs(15), |e| e.document.look.is_none());
        app.read(|e| {
            assert_eq!(
                e.document.state.project.output.look.as_ref().unwrap(),
                &accepted
            );
            assert_eq!(e.look.loaded.len(), 1);
            assert!(e.failure.is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn user_cancel_retires_queued_work_without_changing_the_accepted_look() {
    let (mut e, scheduler) = editor();
    e.document.state.project.output.look = Some(asset(0.2));
    let before = e.document.state.project.clone();
    let hold = crate::background::testing::pause();
    with_application(e, |app| {
        let activity = app.enter(|e, cx| {
            e.select_look(Source::Loaded(asset(0.7)), cx);
            e.document.look.as_ref().unwrap().activity
        });
        app.pump();
        app.update(|e| e.cancel_task(TaskRequest::Look(activity)));
        app.read(|e| {
            assert!(!e.busy && e.document.look.is_none() && e.tasks.view().is_none());
            assert_eq!(e.document.state.project, before);
            assert!(e.look.loaded.is_empty());
        });
        drop(hold);
        app.pump();
        app.read(|e| assert_eq!(e.document.state.project, before));
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
        app.enter(|e, access| e.select_look(Source::Loaded(old), access));
        app.pump();
        app.enter(|e, access| {
            e.select_look(Source::Loaded(chosen.clone()), access);

            let view = e.request_id();
            e.replace_photo_scope(view);
        });
        app.pump();
        assert!(weak.upgrade().is_none());
        app.read(|e| assert!(e.busy && e.document.look.is_some()));
        drop(hold);
        app.run_until(Duration::from_secs(15), |e| e.document.look.is_none());
        app.read(|e| {
            assert_eq!(e.document.state.project.output.look, Some(chosen.clone()));
            assert_eq!(e.look.loaded, vec![chosen]);
            assert!(e.failure.is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn document_transition_retires_look_but_keeps_session_comparison_assets() {
    let (mut e, scheduler) = editor();
    let previous = asset(0.2);
    e.document.state.project.output.look = Some(previous.clone());
    e.remember_imported_look();
    let hold = crate::background::testing::pause();
    with_application(e, |app| {
        app.enter(|e, access| e.select_look(Source::Loaded(asset(0.9)), access));
        app.pump();
        app.enter(|e, access| {
            e.start_document(Preparation::New, access);
        });
        let operation = app.read(|e| e.document_pending);
        app.read(|e| {
            assert_eq!(e.document_pending, operation);
            assert!(e.busy);
            assert_eq!(e.document.state.project.output.look, Some(previous.clone()));
        });
        drop(hold);
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert!(e.document.state.project.output.look.is_none());
            assert_eq!(e.look.loaded, vec![previous]);
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn owner_and_application_retirement_release_unpolled_and_queued_inputs() {
    for application in [false, true] {
        for polled in [false, true] {
            let (e, scheduler) = editor();
            let hold = crate::background::testing::pause();
            let selected = asset(0.8);
            let weak = Arc::downgrade(&selected);
            with_application(e, |app| {
                app.enter(|e, cx| e.select_look(Source::Loaded(selected), cx));
                if polled {
                    app.pump();
                }
                if application {
                    app.close_execution();
                } else {
                    app.update(|e| e.document = Document::default().into());
                    app.pump();
                }
                assert!(weak.upgrade().is_none());
                app.read(|e| assert!(e.document.state.project.output.look.is_none()));
                drop(hold);
            });
            scheduler.close_and_wait().unwrap();
        }
    }
}

#[test]
fn retired_running_bake_keeps_inputs_without_blocking_an_independent_look() {
    let (e, scheduler) = editor();
    let cache = Arc::downgrade(&e.look.services.as_ref().unwrap().cache);
    let directory = e
        .look
        .services
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
            e.select_look(Source::Builtin(Builtin::Kodak2383), access);
        });
        app.pump();
        starts.recv_timeout(Duration::from_secs(15)).unwrap();
        let selected = asset(0.6);
        app.enter(|e, access| {
            e.retire_look_selection();
            e.look.services = None;
            let source = e.source_services.as_ref().unwrap();
            e.look.services = Some(Services::new(
                source.device.clone(),
                source.queue.clone(),
                e.submissions.clone(),
            ));
            e.select_look(Source::Loaded(selected.clone()), access);
        });
        app.run_until(Duration::from_secs(20), |e| e.document.look.is_none());
        assert!(cache.upgrade().is_some() && directory.exists());
        app.read(|e| assert_eq!(e.document.state.project.output.look, Some(selected.clone())));
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
            assert_eq!(e.document.state.project.output.look, Some(selected));
            assert!(e.failure.is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn builtin_selection_reuses_service_cache_without_initializing_the_photographic_renderer() {
    let (e, scheduler) = editor();
    with_application(e, |app| {
        app.enter(|e, access| {
            e.select_look(Source::Builtin(Builtin::Kodak2383), access);
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
            assert!(
                !e.source_services
                    .as_ref()
                    .unwrap()
                    .renderers
                    .is_initialized()
            );
            look
        });
        app.enter(|e, access| {
            e.start_document(Preparation::New, access);
        });
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.update(|e| {
            assert!(e.document.state.project.output.look.is_none());
            let source = e.source_services.as_ref().unwrap();
            let services = e.look.services.as_mut().unwrap();
            Arc::get_mut(&mut services.cache).unwrap().baker = crate::looks::bake::Baker::new(
                source.device.clone(),
                source.queue.clone(),
                e.submissions.clone(),
            );
            assert!(!services.cache.baker.is_initialized());
        });
        app.enter(|e, access| {
            e.select_look(Source::Builtin(Builtin::Kodak2383), access);
        });
        app.run_until(Duration::from_secs(30), |e| e.document.look.is_none());
        app.read(|e| assert_eq!(e.document.state.project.output.look, Some(accepted)));
        app.read(|e| {
            assert!(
                !e.look
                    .services
                    .as_ref()
                    .unwrap()
                    .cache
                    .baker
                    .is_initialized(),
                "the retained cache must satisfy the exact bake specification"
            )
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn unavailable_look_service_restores_idle_feedback_without_changing_the_project() {
    let mut e = fixture();
    e.sync_document();
    let before = e.document.state.project.clone();
    with_application(e, |app| {
        app.enter(|e, access| {
            e.select_look(Source::Loaded(asset(0.4)), access);
            assert!(e.busy && e.document.look.as_ref().unwrap().task.is_some());
        });
        app.run_until(Duration::from_secs(5), |e| !e.busy);
        app.read(|e| {
            assert_eq!(e.document.state.project, before);
            assert!(!e.busy && e.tasks.view().is_none());
            assert_eq!(e.failure.as_ref().unwrap().kind, FailureKind::Look);
        });
    });
}
