use super::*;
use crate::{
    application::test_support::with_application,
    editor::tests::{add_ready_fixture, fixture},
};
use std::{cell::Cell, rc::Rc, time::Duration};

struct Dropped(Rc<Cell<bool>>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

struct OwnedInteraction {
    _task: async_executor::Task<()>,
}
impl crate::editor::tool::Tool for OwnedInteraction {
    fn presentation(&self, _editor: &crate::editor::Editor) -> crate::editor::tool::Presentation {
        crate::editor::tool::Presentation {
            identity: crate::editor::ui::ToolIdentity {
                icon: crate::editor::ui::icons::Icon::Pick,
                title: "Owned interaction",
            },
            primary: None,
            dismiss: "Cancel",
            fit_label: "Fit",
            warning: None,
            help: "",
            navigation_help: None,
        }
    }
    fn action(
        &mut self,
        _action: crate::editor::tool::Action,
        _editor: &mut crate::editor::Editor,
        _cx: &crate::application::UiContext<'_, crate::editor::Editor>,
    ) {
    }
}

#[test]
fn captured_accepted_snapshots_do_not_include_or_retain_a_scope_draft() {
    let mut editor = crate::editor::Editor::state();
    let mut edit = editor.edit_project("Rendering settings").unwrap();
    let mut values = edit.values();
    values.roll.exposure = 1.;
    edit.replace(values);
    let snapshot = editor.document.capture();
    assert_eq!(snapshot.project.roll.exposure, 0.);
    assert_eq!(editor.document.project_snapshot().roll.exposure, 1.);
    drop(edit);
    assert_eq!(editor.document.project_snapshot().roll.exposure, 0.);
    assert_eq!(snapshot.project.roll.exposure, 0.);
}

#[test]
fn document_retirement_drops_child_continuations_and_cache_despite_captured_snapshots() {
    let scope = DocumentScope::from(Document::default());
    let snapshot = scope.capture();
    let store = Arc::downgrade(&scope.cache);
    let dropped = Rc::new(Cell::new(false));
    let resumed = Rc::new(Cell::new(false));
    with_application(scope, |app| {
        let access = app.access();
        let (send, result) = futures_channel::oneshot::channel::<()>();
        let guard = Dropped(dropped.clone());
        let continued = resumed.clone();
        let task = access
            .clone()
            .spawn(move |_| async move {
                let _guard = guard;
                let _ = result.await;
                continued.set(true);
                access.update(|_| panic!("retired document continuation resumed"));
            })
            .unwrap();
        app.update(|scope| {
            scope.set_hosted_tool(
                super::super::tool::Placement::Photo,
                Box::new(OwnedInteraction { _task: task }),
            );
        });
        app.pump();
        app.update(|scope| *scope = Document::default().into());
        assert!(store.upgrade().is_none());
        drop(snapshot);
        app.pump();
        assert!(dropped.get());
        assert!(send.send(()).is_err());
        assert!(!resumed.get());
    });
}

#[test]
fn unmatched_tool_accessors_preserve_the_owner_and_taking_it_transfers_its_lifetime() {
    let dropped = Rc::new(Cell::new(false));
    with_application(DocumentScope::from(Document::default()), |app| {
        let guard = Dropped(dropped.clone());
        let task = app
            .access()
            .spawn(move |_| async move {
                let _guard = guard;
                futures_lite::future::pending::<()>().await;
            })
            .unwrap();
        app.update(|scope| {
            assert!(scope.visit.masks.gesture().is_none());
            scope.set_hosted_tool(
                super::super::tool::Placement::Photo,
                Box::new(OwnedInteraction { _task: task }),
            );
        });
        app.pump();
        app.update(|scope| {
            assert!(scope.crop_edit().is_none());
            assert!(scope.visit.masks.gesture().is_none());
            assert!(scope.flat_edit().is_none());
            scope.set_crop_edit(None);
            scope.visit.masks.cancel();
            scope.set_flat_edit(None);
            scope.set_analysis_operation(None);
            scope.clear_dust();
            assert!(scope.visit.hosted_tool.installed().is_some());
        });
        app.pump();
        assert!(!dropped.get());
        let owner = app.update(|scope| std::mem::take(&mut scope.visit.hosted_tool));
        app.read(|scope| assert!(scope.visit.hosted_tool.installed().is_none()));
        app.pump();
        assert!(!dropped.get());
        drop(owner);
        app.pump();
        assert!(dropped.get());
    });
}

#[test]
fn cancelling_input_confirmation_preserves_the_visit_source_and_history() {
    let mut editor = fixture();
    add_ready_fixture(&mut editor);
    editor.document.project.set_gamma(1.25);
    let before = editor.history_snapshot();
    editor.document.project.roll.exposure = 0.25;
    editor.record_edit(before, "Exposure", None);
    let accepted = editor.document.state.project.clone();
    let source = editor
        .document
        .visit
        .canvas
        .ready
        .as_ref()
        .unwrap()
        .developed
        .clone();
    let view = editor.document.visit.view;
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.request_input(crate::processing::InputSettings::Digital, cx));
        app.read(|editor| {
            assert!(editor.dialogs.is_open());
            assert_eq!(editor.document.visit.view, view);
            assert!(Arc::ptr_eq(
                &editor
                    .document
                    .visit
                    .canvas
                    .ready
                    .as_ref()
                    .unwrap()
                    .developed,
                &source
            ));
        });
        app.update(|editor| editor.dialogs.answer_for_test("Cancel"));
        app.run_until(std::time::Duration::from_secs(5), |editor| {
            editor.document.input_change.is_none()
        });
        app.read(|editor| {
            assert_eq!(editor.document.visit.view, view);
            assert!(Arc::ptr_eq(
                &editor
                    .document
                    .visit
                    .canvas
                    .ready
                    .as_ref()
                    .unwrap()
                    .developed,
                &source
            ));
            assert_eq!(editor.document.state.project, accepted);
            assert_eq!(
                editor.history_label(crate::editor::history::Direction::Undo),
                Some("Exposure")
            );
        });
    });
}

#[test]
fn installing_same_document_preserves_visit_but_opening_the_same_data_replaces_it() {
    for replace in [false, true] {
        let mut editor = fixture();
        add_ready_fixture(&mut editor);

        let view = editor.document.visit.view;
        let store = Arc::downgrade(&editor.document.cache);
        let selected = editor.document.visit.current_source;
        let candidate = editor.document.capture();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("same-data.antitone");
        candidate.save(&path, crate::project::Save::New).unwrap();
        with_application(editor, |app| {
            app.enter(|editor, cx| {
                if replace {
                    editor.open_project(path, cx);
                } else {
                    editor.document.install(candidate);
                }
            });
            app.run_until(Duration::from_secs(10), |editor| {
                !editor.files.opening() && !editor.document.visit.source.pending()
            });
            app.read(|editor| {
                if replace {
                    assert_ne!(editor.document.visit.view, view);
                    assert!(
                        editor.document.visit.canvas.ready.is_none(),
                        "fixture has no source services"
                    );
                } else {
                    assert_eq!(editor.document.visit.view, view);
                    assert!(editor.document.visit.canvas.ready.is_some());
                }
                assert_eq!(store.upgrade().is_none(), replace);
                assert_eq!(
                    store.ptr_eq(&Arc::downgrade(&editor.document.cache)),
                    !replace
                );
                assert_eq!(editor.document.visit.current_source, selected);
            });
        });
    }
}

#[test]
fn successful_save_keeps_visit_history_and_later_edits_while_rebasing_the_written_baseline() {
    let mut editor = fixture();
    let before = editor.history_snapshot();
    editor.document.project.roll.exposure = 0.25;
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
    editor.document.project.roll.exposure = 0.5;
    editor.record_edit(before, "Later exposure", None);
    let view = editor.document.visit.view;
    let store = editor.document.cache.clone();
    editor.document.accept_save(saved);
    assert_eq!(editor.document.state.project.roll.exposure, 0.5);
    assert!(editor.document.dirty());
    assert_eq!(
        editor.history_label(crate::editor::history::Direction::Undo),
        Some("Later exposure")
    );
    assert_eq!(editor.document.visit.view, view);
    assert!(Arc::ptr_eq(&editor.document.cache, &store));
}
