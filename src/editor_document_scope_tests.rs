use super::*;
use crate::{
    application::test_support::with_application,
    editor::{
        Pick, Preparation, Prepared, measurement,
        tests::{add_ready_fixture, fixture},
    },
};
use std::{cell::Cell, rc::Rc};

struct Dropped(Rc<Cell<bool>>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

#[test]
fn document_retirement_cancels_child_continuations_and_closes_cache_despite_captured_snapshots() {
    let scope = DocumentScope::from(Document::default());
    let snapshot = scope.capture();
    let store = scope.cache().clone();
    let dropped = Rc::new(Cell::new(false));
    let resumed = Rc::new(Cell::new(false));
    with_application(scope, |app| {
        let access = app.access();
        let (send, result) = futures_channel::oneshot::channel::<()>();
        let guard = Dropped(dropped.clone());
        let continued = resumed.clone();
        let task = access
            .clone()
            .spawn(async move {
                let _guard = guard;
                let _ = result.await;
                continued.set(true);
                access.update(|_| panic!("retired document continuation resumed"));
            })
            .unwrap();
        app.update(|scope| {
            scope.visit.measurement = Some(measurement::Pending {
                kind: Pick::Dmin,
                request: None,
                _task: Some(task),
            })
        });
        app.pump();
        app.update(|scope| *scope = Document::default().into());
        assert!(store.is_closed());
        assert!(!app.read(|scope| scope.cache().is_closed()));
        assert!(
            snapshot.cache().is_closed(),
            "a captured data snapshot cannot keep publication authority alive"
        );
        app.pump();
        assert!(dropped.get());
        assert!(send.send(()).is_err());
        assert!(!resumed.get());
    });
}

#[test]
fn failed_candidate_preserves_the_visit_source_and_history() {
    let mut editor = fixture();
    add_ready_fixture(&mut editor);
    let before = editor.history_snapshot();
    editor.roll.exposure = 0.25;
    editor.record_edit(before, "Exposure", None);
    let accepted = editor.document.project.clone();
    let source = editor
        .document
        .visit
        .ready
        .as_ref()
        .unwrap()
        .developed
        .clone();
    let view = editor.document.visit.view;
    editor.start_document(Preparation::Open("unavailable.antitone".into()));
    let request = editor.document_pending.unwrap();
    assert!(editor.candidate_operation.request.is_some());
    assert_eq!(editor.document.visit.view, view);
    assert!(Arc::ptr_eq(
        &editor.document.visit.ready.as_ref().unwrap().developed,
        &source
    ));
    editor.document_prepared(request, Err("candidate rejected".into()));
    assert_eq!(editor.document.visit.view, view);
    assert!(Arc::ptr_eq(
        &editor.document.visit.ready.as_ref().unwrap().developed,
        &source
    ));
    assert_eq!(editor.document.project, accepted);
    assert_eq!(
        editor.history_label(crate::editor::history::Direction::Undo),
        Some("Exposure")
    );
}

#[test]
fn same_source_correction_preserves_visit_but_opening_the_same_data_replaces_it() {
    for replace in [false, true] {
        let mut editor = fixture();
        add_ready_fixture(&mut editor);
        editor.sync_document();
        let view = editor.document.visit.view;
        let store = editor.document.cache().clone();
        let ready = editor.document.visit.ready.as_ref().unwrap().clone();
        let selected = editor.current_source;
        let mut candidate = editor.document.capture();
        candidate.fork_cache();
        let work = if replace {
            Preparation::Open("same-data.antitone".into())
        } else {
            Preparation::Correction(candidate.clone(), selected)
        };
        editor.start_document(work);
        let request = editor.document_pending.unwrap();
        assert!(editor.candidate_operation.request.is_some());
        editor.document_prepared(
            request,
            Ok(Box::new(Prepared {
                document: candidate,
                ready: Some(ready),
                selected,
                reuse_ready: false,
                correction: None,
                preserve_view: !replace,
            })),
        );
        if replace {
            assert_ne!(editor.document.visit.view, view);
        } else {
            assert_eq!(editor.document.visit.view, view);
        }
        assert!(editor.document.visit.ready.is_some());
        assert!(store.is_closed());
        assert!(!editor.document.cache().is_closed());
        assert_eq!(editor.current_source, selected);
    }
}

#[test]
fn successful_save_keeps_visit_history_and_later_edits_while_rebasing_the_written_baseline() {
    let mut editor = fixture();
    let before = editor.history_snapshot();
    editor.roll.exposure = 0.25;
    editor.record_edit(before, "First exposure", None);
    let directory = tempfile::tempdir().unwrap();
    let saved = editor
        .document
        .capture()
        .save(
            &directory.path().join("written.antitone"),
            crate::project::Save::New,
        )
        .unwrap();
    let before = editor.history_snapshot();
    editor.roll.exposure = 0.5;
    editor.record_edit(before, "Later exposure", None);
    let view = editor.document.visit.view;
    let store = editor.document.cache().clone();
    editor.document.accept_save(saved);
    assert_eq!(editor.document.project.roll.exposure, 0.5);
    assert!(editor.document.dirty());
    assert_eq!(
        editor.history_label(crate::editor::history::Direction::Undo),
        Some("Later exposure")
    );
    assert_eq!(editor.document.visit.view, view);
    assert!(Arc::ptr_eq(editor.document.cache(), &store));
    assert!(!store.is_closed());
}
