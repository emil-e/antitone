use super::*;
use crate::{
    application::test_support::with_application,
    editor::{
        Coverage, Pick, Scan,
        tests::{add_ready_fixture, fixture, hold_blocking_work},
    },
};
use std::{
    cell::Cell,
    rc::Rc,
    sync::{Arc, mpsc},
    time::Duration,
};

#[test]
fn retiring_a_visit_discards_its_manual_crop_without_accepting_or_recording_it() {
    let mut editor = fixture();
    add_ready_fixture(&mut editor);
    editor.sync_document();
    let accepted = editor.document.project.clone();
    let before = editor
        .history_label(crate::editor::history::Direction::Undo)
        .map(str::to_owned);
    editor.begin_crop();
    editor.document.visit.crop_edit.as_mut().unwrap().draft.size = [256, 256];
    let next = editor.request_id();
    editor.replace_photo_scope(next);
    assert!(editor.document.visit.crop_edit.is_none());
    editor.finish_crop(true);
    assert_eq!(editor.document.project, accepted);
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
            let source = editor.current_source.unwrap();
            let id = editor
                .document
                .project
                .source(source)
                .unwrap()
                .local_adjustments
                .as_ref()
                .unwrap()
                .adjustments[0]
                .id;
            editor.select_adjustment(crate::editor::adjustments::Target::Local(id));
            editor.document.visit.adjustments.show_coverage = true;
            if radial {
                editor.begin_radial_mask(None);
            } else {
                editor.begin_linear_mask(None);
            }
            assert!(editor.document.visit.mask_edit.is_some());
            let accepted = editor.document.project.clone();
            let before = editor
                .history_label(crate::editor::history::Direction::Undo)
                .map(str::to_owned);
            let next = editor.request_id();
            editor.replace_photo_scope(next);
            assert!(editor.document.visit.mask_edit.is_none());
            assert!(!editor.document.visit.adjustments.drawer_open());
            assert!(!editor.document.visit.adjustments.show_coverage);
            assert_eq!(
                editor.adjustment_target(),
                crate::editor::adjustments::Target::Global
            );
            editor.finish_mask(true);
            assert_eq!(editor.document.project, accepted);
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
    let hold = hold_blocking_work(&editor.workflow_jobs);
    with_application(editor, |app| {
        app.enter(|editor, access| {
            editor.begin_pick(Pick::Dmin);
            editor.region = Some(Coverage::rectangle(
                editor.document.visit.ready.as_ref().unwrap().area,
            ));
            editor.apply_pick();
            editor.start_measurement_workflow(access);
        });
        app.pump();
        let (view, path) = app.read(|editor| {
            (
                editor.document.visit.view,
                editor.document.visit.ready.as_ref().unwrap().path.clone(),
            )
        });
        app.update(|editor| {
            editor.roll.gamma = 0.;
            editor.load("next.dng".into());
        });
        app.read(|editor| {
            assert_eq!(editor.document.visit.view, view);
            assert!(editor.document.visit.measurement.is_some());
            assert!(editor.busy);
            assert_eq!(editor.id, view);
            assert_eq!(editor.document.visit.ready.as_ref().unwrap().path, path);
        });
    });
    drop(hold);
}

#[test]
fn revisiting_the_same_source_does_not_restore_its_departed_measurement() {
    let mut editor = fixture();
    add_ready_fixture(&mut editor);
    let hold = hold_blocking_work(&editor.workflow_jobs);
    with_application(editor, |app| {
        app.enter(|editor, access| {
            editor.begin_pick(Pick::Dmin);
            editor.region = Some(Coverage::rectangle(
                editor.document.visit.ready.as_ref().unwrap().area,
            ));
            editor.apply_pick();
            editor.start_measurement_workflow(access);
        });
        app.pump();
        app.update(Editor::sync_document);
        let (view, path, project) = app.read(|editor| {
            (
                editor.document.visit.view,
                editor.document.visit.ready.as_ref().unwrap().path.clone(),
                editor.document.project.clone(),
            )
        });
        app.update(|editor| editor.load("next.dng".into()));
        assert!(app.read(|e| e.document.visit.source.request.is_some()));
        app.update(|editor| editor.load(path));
        assert!(app.read(|e| e.document.visit.source.request.is_some()));
        app.pump();
        app.read(|editor| {
            assert_ne!(editor.document.visit.view, view);
            assert!(editor.document.visit.measurement.is_none());
            assert_eq!(editor.document.project, project);
            assert!(editor.busy, "the new visit still owns its source load");
        });
    });
    drop(hold);
}

#[test]
fn leaving_a_visit_cancels_its_continuation_but_not_running_source_computation() {
    let mut editor = fixture();
    add_ready_fixture(&mut editor);
    let source = Arc::downgrade(&editor.document.visit.ready.as_ref().unwrap().developed);
    let continued = Rc::new(Cell::new(false));
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let (finished, finishes) = mpsc::channel();
    with_application(editor, |app| {
        let access = app.access();
        let input = app.read(|editor| {
            let area = Scan::from_source(
                editor
                    .document
                    .visit
                    .ready
                    .as_ref()
                    .unwrap()
                    .developed
                    .clone(),
            )
            .area;
            editor
                .measurement_input(Pick::Dmin, Coverage::rectangle(area))
                .unwrap()
        });
        let jobs = app.read(|editor| editor.workflow_jobs.clone());
        let continuation = continued.clone();
        let view = app.read(|editor| editor.document.visit.view);
        let target = access.clone();
        let task = access
            .spawn(async move {
                let result = jobs
                    .run(move || {
                        entered.send(()).unwrap();
                        let _ = resume.recv();
                        let result = input.measure().map_err(|error| error.to_string());
                        finished.send(()).unwrap();
                        result
                    })
                    .await
                    .unwrap();
                continuation.set(true);
                target.update(|editor| editor.measurement_result(view, Pick::Dmin, result));
            })
            .unwrap();
        app.update(|editor| {
            editor.document.visit.measurement = Some(measurement::Pending {
                kind: Pick::Dmin,
                request: None,
                _task: Some(task),
            });
            editor.busy = true;
        });
        app.pump();
        starts.recv_timeout(Duration::from_secs(5)).unwrap();
        app.update(|editor| editor.load("next.dng".into()));
        assert!(app.read(|e| e.document.visit.source.request.is_some()));
        let status = app.read(|editor| editor.status.clone());
        app.pump();
        assert!(
            source.upgrade().is_some(),
            "running work retains its source after visit teardown"
        );
        release.send(()).unwrap();
        finishes.recv_timeout(Duration::from_secs(5)).unwrap();
        app.run_until(Duration::from_secs(5), |_| source.upgrade().is_none());
        app.pump();
        assert!(
            !continued.get(),
            "the departed visit's continuation must not resume at all"
        );
        app.read(|editor| {
            assert!(editor.document.visit.measurement.is_none());
            assert_eq!(editor.status, status);
            assert!(editor.busy);
        });
    });
}
