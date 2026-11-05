use super::*;
use crate::{
    application::test_support::with_application,
    editor::{
        analysis::workflow_tests::{LIMIT, fixture},
        history::Direction,
        source_loading,
    },
};

#[test]
fn review_projection_is_read_only_and_retirement_preserves_normal_navigation_without_painting() {
    for source in [false, true] {
        let sources = source_loading::SourceLoader::new();
        let mut editor = fixture(sources.clone());
        editor.document.source = source;
        editor.document.visit.canvas.viewport.zoom = Some(1.75);
        let normal_viewport = editor.document.visit.canvas.viewport;
        let before = editor.document.project.clone();
        with_application(editor, |app| {
            app.enter(|editor, cx| editor.request_analysis(cx));
            app.run_until(LIMIT, |editor| editor.document.analysis_review().is_some());
            app.update(|editor| {
                let id = editor.document.visit.current_source.unwrap();
                let active = editor
                    .document
                    .project
                    .source(id)
                    .unwrap()
                    .development
                    .active_area;
                let region = Coverage::rectangle(Rect {
                    x: active[0] + 8,
                    y: active[1] + 8,
                    width: active[2] - 16,
                    height: active[3] - 16,
                });
                editor.set_test_pick_region(Some(region));
                let approved = editor
                    .document
                    .analysis_review()
                    .unwrap()
                    .approved
                    .entries
                    .clone();
                editor.with_tool_viewport(|viewport| viewport.zoom = Some(2.25));
                let draft_viewport = editor.document.analyze_tool().unwrap().viewport;
                for area in [
                    active.into(),
                    Rect {
                        x: 10,
                        y: 20,
                        width: 640,
                        height: 480,
                    },
                ] {
                    let mut roll = RollOrientation::default();
                    roll.turn_clockwise();
                    let photograph =
                        Composition::new(area, roll, PhotoOrientation::default()).unwrap();
                    editor
                        .with_current_tool(|tool, _| {
                            for _ in 0..2 {
                                assert_eq!(
                                    tool.composition(photograph),
                                    Composition::new(
                                        area,
                                        RollOrientation::default(),
                                        PhotoOrientation::default()
                                    )
                                    .unwrap()
                                );
                            }
                        })
                        .unwrap();
                }
                assert_eq!(
                    editor.document.analysis_review().unwrap().approved.entries,
                    approved
                );
                assert_eq!(editor.test_pick_region(), Some(region));
                assert_eq!(
                    editor.document.analyze_tool().unwrap().viewport,
                    draft_viewport
                );
                assert_eq!(editor.document.visit.canvas.viewport, normal_viewport);
                assert_eq!(editor.document.source, source);
                assert_eq!(editor.document.project, before);
                let cancellation = editor
                    .document
                    .analysis_operation()
                    .unwrap()
                    .cancelled
                    .clone();
                editor.end_tool();
                assert!(cancellation.is_cancelled());
                assert!(editor.current_tool().is_none());
                assert_eq!(editor.document.visit.canvas.viewport, normal_viewport);
                assert_eq!(editor.document.source, source);
                assert_eq!(editor.document.project, before);
                assert!(editor.history_label(Direction::Undo).is_none());
            });
        });
        sources.close_and_wait().unwrap();
    }
}

#[test]
fn dismissing_while_joining_shared_regions_retires_work_and_reports_cancellation() {
    let sources = source_loading::SourceLoader::new();
    let mut editor = fixture(sources.clone());
    let before = editor.document.project.clone();
    let (_reply, receive) = futures_channel::oneshot::channel();
    editor.document.spatial_reply = Some(receive);
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.request_analysis(cx);
            assert!(
                editor
                    .document
                    .spatial_preparing
                    .as_ref()
                    .unwrap()
                    .visible()
                    && editor.interaction_blocked()
            );
        });
        app.pump();
        app.update(|editor| editor.hosted_action(tool::Action::Dismiss));
        app.pump();
        app.read(|editor| {
            assert!(editor.current_tool().is_none() && editor.document.spatial_preparing.is_none());
            assert!(!editor.interaction_blocked() && editor.activities.view().is_none());
            assert!(editor.status.contains("cancelled"), "{}", editor.status);
            assert_eq!(editor.document.project, before);
            assert!(editor.history_label(Direction::Undo).is_none());
        });
    });
    sources.close_and_wait().unwrap();
}
