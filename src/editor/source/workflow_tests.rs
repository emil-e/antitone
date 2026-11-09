use super::*;
use crate::processing::InputSettings;

#[test]
fn selected_photo_preparation_does_not_block_edits_and_publication_preserves_them() {
    let (scheduler, starts, release) = paused();
    let mut editor = editor(scheduler.clone());
    editor.load_input(InputSettings::Digital);
    with_application(editor, |app| {
        start(app);
        await_native(app, &starts);
        let (selected, orientation, crop) = app.enter(|editor, cx| {
            assert!(editor.document.visit.source.pending());
            assert!(editor.document.visit.canvas.ready.is_none());
            assert!(!editor.interaction_blocked());
            let selected = editor.document.visit.current_source.unwrap();
            let menu = editor.application_menu_state();
            for action in [
                crate::platform::menu::Action::Export,
                crate::platform::menu::Action::Fit,
                crate::platform::menu::Action::ActualSize,
                crate::platform::menu::Action::ZoomIn,
                crate::platform::menu::Action::ZoomOut,
            ] {
                assert!(menu.enabled.contains(&action), "{action:?}");
            }
            editor.begin_linear_mask(None);
            assert!(editor.has_hosted_tool());
            editor.end_tool();
            editor.begin_radial_mask(None);
            assert!(editor.has_hosted_tool());
            editor.end_tool();
            editor.begin_dust_cleanup();
            assert!(editor.dusting());
            editor.end_tool();
            let previous = editor.photo_orientation();
            editor.apply_photo_action(crate::editor::editing::PhotoAction::TurnRight, cx);
            assert_ne!(editor.photo_orientation(), previous);
            let before = editor.history_snapshot();
            editor.photo_settings_mut().exposure = 0.75;
            editor.record_edit(before, "Exposure", Some(selected));
            editor.begin_crop();
            let area = editor.selected_area().unwrap();
            let mut crop = crate::crop::Crop::from_region(area);
            crop.size[0] -= 8;
            editor.document.crop_edit_mut().unwrap().draft = crop;
            editor.dispatch_tool(crate::editor::tool::Action::Primary, cx);
            assert!(!editor.cropping());
            assert_eq!(editor.photo_crop(), Some(crop));
            (selected, editor.photo_orientation(), crop)
        });
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(15), |editor| {
            !editor.document.visit.source.pending()
        });
        app.read(|editor| {
            assert!(editor.document.visit.canvas.ready.is_some());
            assert_eq!(editor.document.visit.current_source, Some(selected));
            assert_eq!(editor.photo_orientation(), orientation);
            assert_eq!(editor.photo_crop(), Some(crop));
            assert_eq!(editor.photo_settings().exposure, 0.75);
            let source = editor.document.project.source(selected).unwrap();
            assert_eq!(source.orientation, orientation);
            assert_eq!(source.crop, Some(crop));
            assert_eq!(
                source.adjustment_layers.as_ref().unwrap().layers[0]
                    .adjustment
                    .photo
                    .exposure,
                0.75
            );
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn a_picker_opened_during_photo_preparation_waits_for_its_captured_precise_input() {
    let (scheduler, starts, release) = paused();
    let mut editor = editor(scheduler.clone());
    editor.load_input(InputSettings::negative(crate::processing::Calibration {
        dmin: glam::DVec3::splat(0.7),
        dmax: glam::DVec3::splat(0.01),
    }));
    with_application(editor, |app| {
        start(app);
        await_native(app, &starts);
        app.enter(|editor, cx| {
            assert!(!editor.interaction_blocked());
            editor.begin_test_picker(crate::editor::PickerCase::Dmin, cx);
        });
        app.pump();
        let previous = app.enter(|editor, _| {
            assert!(editor.picking());
            editor.set_test_pick_region(Some(crate::coverage::Coverage::rectangle(
                crate::raw::Rect {
                    x: 24,
                    y: 24,
                    width: 16,
                    height: 16,
                },
            )));
            assert!(editor.submit_test_picker());
            assert!(!editor.picking());
            editor.dmin()
        });
        app.pump();
        app.read(|editor| {
            assert!(editor.measurement_pending() && editor.interaction_blocked());
            assert_eq!(editor.dmin(), previous);
            assert!(editor.document.visit.source.pending());
        });
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(15), |editor| {
            !editor.measurement_pending() && !editor.document.visit.source.pending()
        });
        app.read(|editor| {
            let actual = editor.dmin().unwrap();
            assert!(
                (actual - glam::DVec3::splat(3000. / 4031.))
                    .abs()
                    .max_element()
                    < 1e-5
            );
            assert!(!editor.interaction_blocked());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn cancelling_measurement_waiting_for_source_withdraws_only_its_acquisition() {
    let (scheduler, starts, release) = paused();
    let mut editor = editor(scheduler.clone());
    editor.load_input(InputSettings::negative(crate::processing::Calibration {
        dmin: glam::DVec3::splat(0.7),
        dmax: glam::DVec3::splat(0.01),
    }));
    with_application(editor, |app| {
        start(app);
        await_native(app, &starts);
        let before = app.read(|editor| editor.document.project.clone());
        app.enter(|editor, cx| {
            editor
                .start_interaction(cx, crate::editor::calibration::pick_dmin)
                .unwrap();
        });
        app.pump();
        app.enter(|editor, cx| {
            editor.set_test_pick_region(Some(crate::coverage::Coverage::rectangle(
                crate::raw::Rect {
                    x: 24,
                    y: 24,
                    width: 16,
                    height: 16,
                },
            )));
            editor.apply_pick(cx);
        });
        // No paint/thumbnail demand runs in this fixture: these are the selected source
        // and the picker joining the same held native acquisition.
        app.run_until(Duration::from_secs(5), |_| scheduler.waiter_count() == 2);
        app.update(|editor| editor.activities.view().unwrap().request_cancel());
        app.run_until(Duration::from_secs(5), |_| scheduler.waiter_count() == 1);
        app.read(|editor| {
            assert!(!editor.picking() && !editor.interaction_blocked());
            assert!(editor.document.visit.source.pending());
            assert_eq!(editor.document.project, before);
        });
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(15), |editor| {
            !editor.document.visit.source.pending()
        });
        app.read(|editor| {
            assert!(editor.document.visit.canvas.ready.is_some());
            assert!(!editor.interaction_blocked());
            assert_eq!(editor.document.project, before);
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn unregistered_source_requests_defer_file_identity_until_preparation() {
    let editor = Editor::state();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng");
    for request in [
        editor.source_input(path.clone()),
        editor.reference_preview_input(path.clone()),
    ] {
        assert!(request.expected.is_none());
        assert!(
            request.file.revision.is_none(),
            "UI capture must leave first-time filesystem identity to preparation"
        );
        let key =
            futures_lite::future::block_on(crate::background::run(move |_| request.load_key()))
                .unwrap()
                .unwrap();
        assert!(
            key.file().revision.is_some(),
            "preparation must capture persisted-cache identity"
        );
    }
}

use crate::{
    application::test_support::with_application,
    editor::{Preparation, Prepared, document::Document, source, source_loading},
};
use std::{
    path::PathBuf,
    sync::{Mutex, mpsc},
    time::Duration,
};

fn path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng")
}

fn editor(scheduler: source_loading::SourceLoader) -> Editor {
    let mut editor = super::super::tests::fixture();
    let mut document = Document::default();
    document.import(&[path()]).unwrap();
    editor.document.install(document);
    editor
        .document
        .thumbnails
        .set_limits(super::super::cache_limits::CacheLimits {
            cpu_bytes: 0,
            gpu_bytes: 0,
        });
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
    editor.source_services = Some(source::Services {
        developments: scheduler,
        renderers: Arc::default(),
        device,
        queue,
    });
    editor
}

fn paused() -> (
    source_loading::SourceLoader,
    mpsc::Receiver<()>,
    mpsc::Sender<()>,
) {
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Mutex::new(resume);
    let scheduler = source_loading::SourceLoader::new_observed(move |_, phase, _| {
        if matches!(phase, source_loading::NativePhase::UnpackBegin) {
            entered.send(()).unwrap();
            resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(15))
                .unwrap();
        }
        Ok(())
    });
    (scheduler, starts, release)
}

fn start(app: &crate::application::test_support::TestApplication<'_, Editor>) {
    app.enter(|editor, access| {
        editor.load(path(), access);
    });
}

fn await_native(
    app: &mut crate::application::test_support::TestApplication<'_, Editor>,
    entered: &mpsc::Receiver<()>,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        app.pump();
        if entered.try_recv().is_ok() {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "native source did not reach its barrier"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn retiring_source_ownership_does_not_clear_an_unrelated_document_activity() {
    let editor = super::super::tests::fixture();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.load("not-opened.dng".into(), cx);
            editor.start_document(Preparation::New, cx);
            let operation = editor.document_pending;
            let activity = editor.activities.view().unwrap().title;
            let view = editor.request_id();
            editor.replace_photo_scope(view);
            assert!(editor.interaction_blocked());
            assert_eq!(editor.document_pending, operation);
            assert_eq!(editor.activities.view().unwrap().title, activity);
        });
    });
}

#[test]
fn requested_source_becomes_ready_without_painting_and_preserves_authored_pixels() {
    let scheduler = source_loading::SourceLoader::new();
    let editor = editor(scheduler.clone());
    with_application(editor, |app| {
        start(app);
        app.run_until(Duration::from_secs(15), |e| {
            !e.document.visit.source.pending()
        });
        app.read(|e| {
            let ready = e.document.visit.canvas.ready.as_ref().unwrap();
            assert_eq!(ready.path, path());
            assert!(!e.document.visit.source.pending());
            assert!(e.activities.view().is_none());
            assert_eq!(e.document.cache.charges().retained, 0);
            let mut scan = crate::scan::Scan::from_source(ready.developed.clone());
            let value = scan
                .reference(crate::raw::Rect {
                    x: 24,
                    y: 24,
                    width: 16,
                    height: 16,
                })
                .unwrap();
            assert!(
                (value - glam::DVec3::splat(3000. / 4031.))
                    .abs()
                    .max_element()
                    <= 2. / 65535.
            );
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn retired_visit_cannot_publish_or_block_unrelated_cpu_work() {
    let (scheduler, starts, release) = paused();
    let editor = editor(scheduler.clone());
    with_application(editor, |app| {
        start(app);
        await_native(app, &starts);
        app.update(|e| {
            let view = e.request_id();
            e.id = view;
            e.replace_photo_scope(view);
            e.status = "new visit".into();
        });
        app.pump();
        assert_eq!(scheduler.waiter_count(), 0);
        let (entered, observed) = mpsc::channel();
        let access = app.access();
        let probe = access
            .spawn(move |_| async move {
                crate::background::run(move |_| entered.send(()).unwrap())
                    .await
                    .unwrap();
            })
            .unwrap();
        app.run_until(Duration::from_secs(10), |_| observed.try_recv().is_ok());
        release.send(()).unwrap();
        drop(probe);
        app.read(|e| {
            assert!(e.document.visit.canvas.ready.is_none());
            assert_eq!(e.status, "new visit");
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn cancelling_an_unstarted_native_demand_does_not_wait_for_unrelated_calls() {
    let (scheduler, starts, release) = paused();
    let mut backgrounds = Vec::new();
    for _ in 0..2 {
        backgrounds.push(
            scheduler
                .submit(
                    source_loading::Request {
                        path: path(),
                        wb: None,
                        expected: None,
                        key: None,
                    },
                    source_loading::Priority::Eager,
                )
                .unwrap(),
        );
    }
    // Both native owners are inside unrelated calls, so the demand below
    // stays unstarted; cancelling its waiter must not wait for those calls.
    starts.recv_timeout(Duration::from_secs(10)).unwrap();
    starts.recv_timeout(Duration::from_secs(10)).unwrap();
    let editor = editor(scheduler.clone());
    with_application(editor, |app| {
        start(app);
        app.run_until(Duration::from_secs(10), |_| scheduler.waiter_count() == 3);
        app.update(|e| {
            let view = e.request_id();
            e.replace_photo_scope(view);
        });
        app.pump();
        assert_eq!(scheduler.waiter_count(), 2);
        let (entered, observed) = mpsc::channel();
        let probe = app
            .access()
            .spawn(move |_| async move {
                crate::background::run(move |_| entered.send(()).unwrap())
                    .await
                    .unwrap();
            })
            .unwrap();
        app.run_until(Duration::from_secs(10), |_| observed.try_recv().is_ok());
        drop(probe);
    });
    release.send(()).unwrap();
    release.send(()).unwrap();
    drop(backgrounds);
    scheduler.close_and_wait().unwrap();
}

#[test]
fn worker_closure_cleans_up_parked_sources_without_blocking_unrelated_cpu_work() {
    let (parked, at_park) = mpsc::channel();
    let (continue_a, resume_a) = mpsc::channel();
    let resume_a = Mutex::new(resume_a);
    let (entered_b, at_b) = mpsc::channel();
    let (continue_b, resume_b) = mpsc::channel();
    let resume_b = Mutex::new(resume_b);
    let scheduler = source_loading::SourceLoader::new_observed(move |file, phase, _| {
        if file.ends_with("patches.dng") && matches!(phase, source_loading::NativePhase::UnpackEnd)
        {
            parked.send(()).unwrap();
            resume_a
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(15))
                .unwrap();
        }
        if file.ends_with("roll-0.dng") && matches!(phase, source_loading::NativePhase::UnpackBegin)
        {
            entered_b.send(()).unwrap();
            resume_b
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(15))
                .unwrap();
        }
        Ok(())
    });
    let editor = editor(scheduler.clone());
    with_application(editor, |app| {
        start(app);
        await_native(app, &at_park);
        app.update(|e| {
            let view = e.request_id();
            e.replace_photo_scope(view);
        });
        app.pump();
        // Two unrelated Visible calls occupy both native owners so the unpacked
        // source stays parked between phases, still holding mandatory backing.
        let other = path().with_file_name("roll-0.dng");
        let mut tickets = Vec::new();
        for _ in 0..2 {
            tickets.push(
                scheduler
                    .submit(
                        source_loading::Request {
                            path: other.clone(),
                            wb: None,
                            expected: None,
                            key: None,
                        },
                        source_loading::Priority::Visible,
                    )
                    .unwrap(),
            );
        }
        continue_a.send(()).unwrap();
        for _ in 0..2 {
            at_b.recv_timeout(Duration::from_secs(10)).unwrap();
        }
        scheduler.close();
        let (entered, observed) = mpsc::channel();
        let probe = app
            .access()
            .spawn(move |_| async move {
                crate::background::run(move |_| entered.send(()).unwrap())
                    .await
                    .unwrap();
            })
            .unwrap();
        app.run_until(Duration::from_secs(10), |_| observed.try_recv().is_ok());
        continue_b.send(()).unwrap();
        continue_b.send(()).unwrap();
        drop((probe, tickets));
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn worker_closure_finishes_the_waiter_while_native_cleanup_continues() {
    let (scheduler, starts, release) = paused();
    let editor = editor(scheduler.clone());
    with_application(editor, |app| {
        start(app);
        await_native(app, &starts);
        scheduler.close();
        app.run_until(Duration::from_secs(10), |e| {
            !e.document.visit.source.pending()
        });
        assert!(app.read(|e| e.document.visit.canvas.ready.is_none()));
        let (entered, observed) = mpsc::channel();
        let probe = app
            .access()
            .spawn(move |_| async move {
                crate::background::run(move |_| entered.send(()).unwrap())
                    .await
                    .unwrap();
            })
            .unwrap();
        app.run_until(Duration::from_secs(10), |_| observed.try_recv().is_ok());
        release.send(()).unwrap();
        drop(probe);
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn failed_candidate_keeps_pending_source_and_defers_its_completion_without_losing_it() {
    let (scheduler, starts, release) = paused();
    let editor = editor(scheduler.clone());
    let directory = tempfile::tempdir().unwrap();
    with_application(editor, |app| {
        start(app);
        await_native(app, &starts);
        let (view, accepted) =
            app.read(|e| (e.document.visit.view, e.document.state.project.clone()));
        app.enter(|e, cx| {
            e.start_document(
                Preparation::Open(directory.path().join("missing.antitone")),
                cx,
            )
        });
        assert!(app.read(|e| e.document_pending.is_some() && e.interaction_blocked()));
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(15), |e| {
            e.document_pending.is_none()
                && !e.interaction_blocked()
                && !e.document.visit.source.pending()
        });
        app.read(|e| {
            assert_eq!(e.document.visit.view, view);
            assert!(e.document.visit.canvas.ready.is_some());
            assert_eq!(e.document.state.project, accepted);
            assert!(e.activities.view().is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn typed_candidate_and_source_completions_preserve_the_visit_in_both_orders() {
    for complete_first in [false, true] {
        let editor = super::super::tests::authored_fixture(&["patches.dng"]);
        let ready = editor.document.visit.canvas.ready.as_ref().unwrap().clone();
        let path = ready.path.clone();
        let accepted = editor.document.state.project.clone();
        with_application(editor, |app| {
            app.enter(|e, cx| {
                e.load(path, cx);

                let view = e.document.visit.view;
                e.start_document(Preparation::Open("not-prepared.antitone".into()), cx);
                let operation = e.document_pending.unwrap();
                if complete_first {
                    e.source_result(view, Ok(Box::new(ready.clone())));
                    assert!(e.document.visit.source.completion.is_some());
                    assert!(e.document.visit.canvas.ready.is_none() && e.interaction_blocked());
                }
                e.document_prepared(operation, Err("candidate rejected".into()));
                if !complete_first {
                    assert!(e.document.visit.source.pending());
                    e.source_result(view, Ok(Box::new(ready)));
                }
                assert_eq!(e.document.visit.view, view);
                assert!(e.document.visit.canvas.ready.is_some() && e.document_pending.is_none());
                assert_eq!(e.document.state.project, accepted);
                assert!(e.status.contains("candidate rejected"));
            });
            app.pump();
            app.read(|e| assert!(e.activities.view().is_none()));
        });
    }
}

#[test]
fn save_preserves_pending_visit_and_later_edits_in_both_completion_orders() {
    // Typed-outcome ordering is a controller test. Worker-lifetime cases above
    // exercise physical source execution; this case does not simulate that timing.
    for complete_first in [false, true] {
        let editor = super::super::tests::authored_fixture(&["patches.dng"]);
        let ready = editor.document.visit.canvas.ready.as_ref().unwrap().clone();
        let source_path = ready.path.clone();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("written.antitone");
        with_application(editor, |app| {
            app.enter(|e, cx| {
                e.load(source_path, cx);

                let view = e.document.visit.view;
                let snapshot = e.document.capture();
                e.save_project(path.clone(), crate::project::Save::New, None, cx);
                let operation = e.document_pending.unwrap();
                let saved = snapshot.save(&path, crate::project::Save::New).unwrap();
                e.document.project.roll.exposure = 0.5;

                if complete_first {
                    e.source_result(view, Ok(Box::new(ready.clone())));
                    assert!(e.document.visit.source.completion.is_some());
                    assert!(e.document.visit.canvas.ready.is_none() && e.interaction_blocked());
                }
                e.save_result(operation, Ok(Box::new(saved)), cx);
                if !complete_first {
                    assert!(e.document.visit.source.pending());
                    e.source_result(view, Ok(Box::new(ready)));
                }
                assert_eq!(e.document.visit.view, view);
                assert!(e.document.visit.canvas.ready.is_some() && e.document_pending.is_none());
                assert!(e.document.dirty());
                assert_eq!(e.document.state.project.roll.exposure, 0.5);
                assert_eq!(e.document.path.as_deref(), Some(path.as_path()));
            });
            app.pump();
            app.read(|e| assert!(e.activities.view().is_none()));
        });
    }
}

#[test]
fn a_correction_preparation_retires_the_pending_visit_and_installs_its_product() {
    // A flat-field correction changes the source's development, so the product being acquired is
    // already stale: the pending visit is retired and the correction acquires its own.
    let (scheduler, starts, release) = paused();
    let editor = editor(scheduler.clone());
    with_application(editor, |app| {
        start(app);
        await_native(app, &starts);
        let view = app.read(|e| e.document.visit.view);
        assert!(app.read(|e| e.document.visit.source.pending()));
        app.enter(|e, cx| {
            e.start_document(
                Preparation::Correction(e.document.capture(), e.document.visit.current_source),
                cx,
            )
        });
        let operation = app.read(|e| e.document_pending.unwrap());
        assert!(app.read(|e| e.interaction_blocked() && e.document_pending.is_some()));
        // Release the held acquisition and let the correction's own development run.
        for _ in 0..8 {
            let _ = release.send(());
        }
        app.run_until(Duration::from_secs(30), |e| {
            !e.interaction_blocked() && e.document_pending.is_none()
        });
        app.read(|e| {
            assert_eq!(e.id, operation);
            // The pending acquisition is retired rather than left dangling, and the document's
            // own product is installed.
            assert!(!e.document.visit.source.pending());
            assert!(e.document.visit.canvas.ready.is_some());
            // Whether the scope survives depends on the prepared product's key, not on the
            // preparation being a correction: this fixture has no reference frame, so the
            // development is unchanged and the visit continues into the installed document.
            // A correction that changes the gains changes the key and replaces the scope.
            assert_eq!(e.document.visit.view, view);
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn replacing_document_retires_pending_source_but_not_its_running_native_call() {
    let (scheduler, starts, release) = paused();
    let editor = editor(scheduler.clone());
    with_application(editor, |app| {
        start(app);
        await_native(app, &starts);
        let store = app.read(|e| Arc::downgrade(&e.document.cache));
        app.enter(|e, cx| e.start_document(Preparation::New, cx));
        let operation = app.read(|e| e.document_pending.unwrap());
        assert!(app.read(|e| e.interaction_blocked() && e.document_pending.is_some()));
        app.update(|editor| {
            editor.document_prepared(
                operation,
                Ok(Box::new(Prepared {
                    document: Document::default(),
                    ready: None,
                    selected: None,
                    reuse_ready: false,
                    correction: None,
                    preserve_view: false,
                })),
            )
        });
        app.pump();
        assert!(store.upgrade().is_none());
        assert_eq!(scheduler.waiter_count(), 0);
        let (entered, observed) = mpsc::channel();
        let probe = app
            .access()
            .spawn(move |_| async move {
                crate::background::run(move |_| entered.send(()).unwrap())
                    .await
                    .unwrap();
            })
            .unwrap();
        app.run_until(Duration::from_secs(10), |_| observed.try_recv().is_ok());
        release.send(()).unwrap();
        drop(probe);
        app.read(|e| {
            assert!(e.document.state.project.sources.is_empty());
            assert!(e.document.visit.canvas.ready.is_none());
            assert!(!e.interaction_blocked() && e.activities.view().is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}
