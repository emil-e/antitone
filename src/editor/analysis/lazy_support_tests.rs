use super::*;

fn state() -> Editor {
    let mut editor = crate::editor::tests::fixture();
    crate::editor::tests::add_ready_fixture(&mut editor);
    editor
}

fn request(editor: &Editor) -> (u64, u64) {
    let pending = editor.document.visit.support.pending.as_ref().unwrap();
    (pending.id, pending.loaded)
}

#[test]
fn picture_suggestion_is_captured_demand_and_is_transient_until_applied() {
    let editor = state();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            let before = editor.document.state.project.clone();
            let support = Rect {
                x: 100,
                y: 100,
                width: 600,
                height: 400,
            };
            editor.begin_pick(Pick::Analysis, cx);
            let (request, loaded) = request(editor);
            assert!(!editor.busy && editor.document.picking.region.is_none());
            editor.analysis_support_result(request, loaded, Ok(Some(support)));
            assert_eq!(
                editor.document.picking.region,
                Some(Coverage::rectangle(analysis::suggest(support)))
            );
            assert_eq!(editor.document.state.project, before);
            editor.finish_pick(cx);
            editor.begin_pick(Pick::Analysis, cx);
            assert!(
                editor.document.visit.support.pending.is_none(),
                "same ready context already has its suggestion"
            );
            assert_eq!(editor.document.state.project, before);
        });
    });
}

#[test]
fn delayed_suggestion_cannot_overwrite_drawing_cancelled_picker_or_reopened_picker() {
    for action in ["draw", "cancel", "reopen", "source", "document"] {
        let editor = state();
        crate::application::test_support::with_application(editor, |app| {
            app.enter(|editor, cx| {
                editor.begin_pick(Pick::Analysis, cx);
                let (old, loaded) = request(editor);
                let drawn = Coverage::rectangle(Rect {
                    x: 200,
                    y: 200,
                    width: 200,
                    height: 100,
                });
                match action {
                    "draw" => editor.document.picking.region = Some(drawn),
                    "cancel" => editor.finish_pick(cx),
                    "reopen" => {
                        editor.finish_pick(cx);
                        editor.begin_pick(Pick::Analysis, cx);
                        assert_ne!(old, request(editor).0);
                    }
                    "source" => editor.load("another-source.ARW".into(), cx),
                    "document" => editor.document_pending = Some(999),
                    _ => unreachable!(),
                }
                let before = editor.document.state.project.clone();
                editor.analysis_support_result(
                    old,
                    loaded,
                    Ok(Some(Rect {
                        x: 100,
                        y: 100,
                        width: 600,
                        height: 400,
                    })),
                );
                assert_eq!(editor.document.state.project, before);
                if action == "draw" {
                    assert_eq!(editor.document.picking.region, Some(drawn));
                } else {
                    assert!(editor.document.picking.region.is_none());
                    assert_eq!(
                        editor
                            .document
                            .visit
                            .ready
                            .as_ref()
                            .and_then(|r| r.analysis_support),
                        None
                    );
                }
            });
        });
    }
}

#[test]
fn accepted_geometry_and_known_missing_support_require_no_lazy_work() {
    for known in [
        Some(None),
        Some(Some(Rect {
            x: 100,
            y: 100,
            width: 600,
            height: 400,
        })),
    ] {
        let editor = state();
        crate::application::test_support::with_application(editor, |app| {
            app.enter(|editor, cx| {
                editor
                    .document
                    .visit
                    .ready
                    .as_mut()
                    .unwrap()
                    .analysis_support = known;
                editor.begin_pick(Pick::Analysis, cx);
                assert!(editor.document.visit.support.pending.is_none());
                editor.finish_pick(cx);
                assert!(!editor.analysis_request().unwrap().local_support);
            });
        });
    }
    let editor = state();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            assert!(editor.analysis_request().unwrap().local_support);
            editor.document.state.project.analysis_area = Some([100, 100, 500, 300]);
            editor.begin_pick(Pick::Analysis, cx);
            assert!(editor.document.visit.support.pending.is_none());
            assert_eq!(
                editor.document.picking.region,
                Some(Coverage::rectangle(Rect {
                    x: 100,
                    y: 100,
                    width: 500,
                    height: 300
                }))
            );
        });
    });
}

#[test]
fn support_failure_does_not_change_processing_or_poison_later_demand() {
    let editor = state();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            let before = editor.document.state.project.clone();
            editor.begin_pick(Pick::Analysis, cx);
            let (id, loaded) = request(editor);
            editor.analysis_support_result(id, loaded, Err("support unavailable".into()));
            assert!(editor.document.picking.region.is_none());
            assert_eq!(editor.document.state.project, before);
            editor.finish_pick(cx);
            editor.begin_pick(Pick::Analysis, cx);
            assert!(editor.document.visit.support.pending.is_some());
        });
    });
}
