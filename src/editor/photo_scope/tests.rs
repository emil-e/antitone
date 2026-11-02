use super::*;
use crate::{
    application::test_support::with_application,
    editor::{
        Coverage, Pick,
        picking::measurement::testing,
        tests::{add_ready_fixture, fixture},
        tool,
    },
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[test]
fn retiring_a_visit_discards_its_manual_crop_without_accepting_or_recording_it() {
    let mut editor = fixture();
    add_ready_fixture(&mut editor);
    editor.sync_document();
    let accepted = editor.document.state.project.clone();
    let before = editor
        .history_label(crate::editor::history::Direction::Undo)
        .map(str::to_owned);
    editor.begin_crop();
    editor.document.crop_edit_mut().unwrap().draft.size = [256, 256];
    let next = editor.request_id();
    editor.replace_photo_scope(next);
    assert!(editor.document.crop_edit().is_none());
    editor.hosted_action(tool::Action::Primary);
    assert_eq!(editor.document.state.project, accepted);
    assert_eq!(
        editor.history_label(crate::editor::history::Direction::Undo),
        before.as_deref()
    );
}

#[test]
fn retiring_a_visit_discards_local_tool_state_without_changing_accepted_adjustments() {
    pollster::block_on(async {
        for radial in [false, true] {
            let (mut editor, _, _) = crate::editor::processing::tests::fixture().await;
            editor.tab = 1;
            let source = editor.document.visit.current_source.unwrap();
            let id = editor
                .document
                .project
                .source(source)
                .unwrap()
                .adjustment_layers
                .as_ref()
                .unwrap()
                .layers[0]
                .id;
            editor.select_adjustment_layer(crate::editor::adjustment_layers::Target::Layer(id));
            editor.document.visit.adjustment_layers.show_coverage = true;
            if radial {
                editor.begin_radial_mask(None);
            } else {
                editor.begin_linear_mask(None);
            }
            assert!(editor.creating_mask());
            let accepted = editor.document.state.project.clone();
            let before = editor
                .history_label(crate::editor::history::Direction::Undo)
                .map(str::to_owned);
            let next = editor.request_id();
            editor.replace_photo_scope(next);
            assert!(!editor.masking());
            assert!(!editor.adjustment_layer_owns_delete());
            assert!(!editor.document.visit.adjustment_layers.show_coverage);
            assert_eq!(
                editor.adjustment_layer_target(),
                crate::editor::adjustment_layers::Target::Layer(id)
            );
            editor.finish_mask(true);
            assert_eq!(editor.document.state.project, accepted);
            assert_eq!(
                editor.history_label(crate::editor::history::Direction::Undo),
                before.as_deref()
            );
        }
    });
}

#[test]
fn rejected_navigation_keeps_the_current_visit_and_its_measurement() {
    let mut editor = fixture();
    add_ready_fixture(&mut editor);
    let hold = crate::background::testing::pause();
    with_application(editor, |app| {
        app.enter(|editor, access| editor.begin_pick(Pick::Dmin, access));
        app.pump();
        app.enter(|editor, access| {
            editor.set_pick_region(Some(Coverage::rectangle(
                editor.document.visit.canvas.ready.as_ref().unwrap().area,
            )));
            editor.apply_pick(access);
        });
        app.pump();
        let (view, path) = app.read(|editor| {
            (
                editor.document.visit.view,
                editor
                    .document
                    .visit
                    .canvas
                    .ready
                    .as_ref()
                    .unwrap()
                    .path
                    .clone(),
            )
        });
        app.enter(|editor, cx| {
            editor.set_gamma(0.);
            editor.load("next.dng".into(), cx);
        });
        app.read(|editor| {
            assert_eq!(editor.document.visit.view, view);
            assert!(editor.measurement_pending());
            assert!(editor.busy);
            assert_eq!(editor.id, view);
            assert_eq!(
                editor.document.visit.canvas.ready.as_ref().unwrap().path,
                path
            );
        });
    });
    drop(hold);
}

#[test]
fn revisiting_the_same_source_does_not_restore_its_departed_measurement() {
    use std::sync::{Mutex, mpsc};
    let mut editor = crate::editor::tests::with_document_services(fixture());
    add_ready_fixture(&mut editor);
    let held_path = editor
        .document
        .visit
        .canvas
        .ready
        .as_ref()
        .unwrap()
        .path
        .clone();
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Mutex::new(resume);
    // The background pause holds measurement, not the native source owner.
    let loader =
        crate::editor::source_loading::SourceLoader::new_observed(move |path, phase, _| {
            if path == held_path
                && matches!(phase, crate::editor::source_loading::NativePhase::Open)
            {
                entered.send(()).map_err(|error| error.to_string())?;
                resume
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(10))
                    .map_err(|error| error.to_string())?;
                return Err("released synthetic source request".into());
            }
            Ok(())
        });
    editor.source_services.as_mut().unwrap().developments = loader.clone();
    let hold = crate::background::testing::pause();
    with_application(editor, |app| {
        app.enter(|editor, access| editor.begin_pick(Pick::Dmin, access));
        app.pump();
        app.enter(|editor, access| {
            editor.set_pick_region(Some(Coverage::rectangle(
                editor.document.visit.canvas.ready.as_ref().unwrap().area,
            )));
            editor.apply_pick(access);
        });
        app.pump();
        assert!(
            app.read(Editor::measurement_pending),
            "the departing visit must have a pending measurement"
        );
        app.update(Editor::sync_document);
        let (view, path, project) = app.read(|editor| {
            (
                editor.document.visit.view,
                editor
                    .document
                    .visit
                    .canvas
                    .ready
                    .as_ref()
                    .unwrap()
                    .path
                    .clone(),
                editor.document.state.project.clone(),
            )
        });
        app.enter(|editor, cx| editor.load("next.dng".into(), cx));
        assert!(app.read(|e| e.document.visit.source.pending()));
        app.enter(|editor, cx| editor.load(path, cx));
        assert!(app.read(|e| e.document.visit.source.pending()));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            app.pump();
            if starts.try_recv().is_ok() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "source request did not reach its native gate"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        app.pump();
        app.read(|editor| {
            assert_ne!(editor.document.visit.view, view);
            assert!(!editor.measurement_pending());
            assert!(editor.document.visit.source.pending());
            assert_eq!(editor.document.state.project, project);
            assert!(editor.busy, "the new visit still owns its source load");
        });
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(5), |editor| {
            !editor.document.visit.source.pending()
        });
        app.read(|editor| {
            assert!(!editor.busy);
            assert!(!editor.measurement_pending());
            assert_eq!(editor.document.state.project, project);
        });
    });
    drop(hold);
    loader.close_and_wait().unwrap();
}

#[test]
fn successful_source_revisit_cannot_restore_departed_measurement_results() {
    let editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    let hold = crate::background::testing::pause();
    with_application(editor, |app| {
        app.enter(|editor, access| editor.begin_pick(Pick::Dmin, access));
        app.pump();
        app.enter(|editor, access| {
            editor.set_pick_region(Some(Coverage::rectangle(
                editor.document.visit.canvas.ready.as_ref().unwrap().area,
            )));
            editor.apply_pick(access);
        });
        app.pump();
        app.read(|editor| {
            assert!(
                editor.measurement_pending(),
                "control: an actual measurement must be waiting before departure"
            )
        });
        app.update(Editor::sync_document);
        let (view, path, project) = app.read(|editor| {
            (
                editor.document.visit.view,
                editor
                    .document
                    .visit
                    .canvas
                    .ready
                    .as_ref()
                    .unwrap()
                    .path
                    .clone(),
                editor.document.state.project.clone(),
            )
        });
        app.enter(|editor, cx| editor.load("next.dng".into(), cx));
        assert!(app.read(|e| e.document.visit.source.pending()));
        app.enter(|editor, cx| editor.load(path.clone(), cx));
        assert!(app.read(|e| e.document.visit.source.pending()));
        app.pump();
        app.read(|editor| {
            assert_ne!(editor.document.visit.view, view);
            assert!(!editor.measurement_pending());
            assert_eq!(editor.document.state.project, project);
            // A cache hit can finish before the paused measurement pool is released.
            assert_eq!(editor.busy, editor.document.visit.source.pending());
            if !editor.document.visit.source.pending() {
                let ready = editor
                    .document
                    .visit
                    .canvas
                    .ready
                    .as_ref()
                    .unwrap_or_else(|| {
                        panic!("revisit did not produce a source: {}", editor.status)
                    });
                assert_eq!(ready.path, path);
            }
        });
        drop(hold);
        app.run_until(Duration::from_secs(15), |editor| {
            !editor.document.visit.source.pending()
        });
        app.read(|editor| {
            assert_eq!(
                editor
                    .document
                    .visit
                    .canvas
                    .ready
                    .as_ref()
                    .unwrap_or_else(|| panic!("revisit failed: {}", editor.status))
                    .path,
                path
            );
            assert!(!editor.measurement_pending());
            assert_eq!(editor.document.state.project, project);
        });
    });
}

#[test]
fn leaving_a_visit_retires_its_real_measurement_without_releasing_running_inputs_early() {
    let mut editor = crate::editor::tests::with_document_services(fixture());
    add_ready_fixture(&mut editor);
    editor.sync_document();
    editor.document.cache.set_budget(0);
    let before = editor.document.state.project.clone();
    let source = Arc::downgrade(
        &editor
            .document
            .visit
            .canvas
            .ready
            .as_ref()
            .unwrap()
            .developed,
    );
    let gate = testing::Gate::install(&mut editor);
    let directory = tempfile::tempdir().unwrap();
    let next_path = directory.path().join("missing.dng");
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.begin_pick(Pick::Dmin, cx));
        app.pump();
        app.enter(|editor, cx| {
            editor.set_pick_region(Some(Coverage::rectangle(
                editor.document.visit.canvas.ready.as_ref().unwrap().area,
            )));
            editor.apply_pick(cx);
        });
        let call = gate.next(app);
        app.enter(|editor, cx| editor.load(next_path, cx));
        app.pump();
        assert!(call.cancelled.is_cancelled());
        assert!(
            source.upgrade().is_some(),
            "running work retains its source after visit teardown"
        );
        assert!(app.read(|editor| editor.active_pick().is_none() && !editor.measurement_pending()));
        call.run();
        let deadline = Instant::now() + Duration::from_secs(5);
        while source.upgrade().is_some() {
            app.pump();
            assert!(
                Instant::now() < deadline,
                "retired measurement kept its input"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        app.run_until(Duration::from_secs(5), |editor| !editor.busy);
        app.read(|editor| {
            assert!(!editor.measurement_pending());
            assert!(editor.active_pick().is_none());
            assert!(!editor.document.visit.source.pending());
            assert!(editor.document.visit.canvas.ready.is_none());
            assert!(editor.tasks.view().is_none());
            assert_eq!(editor.document.state.project, before);
        });
    });
}
