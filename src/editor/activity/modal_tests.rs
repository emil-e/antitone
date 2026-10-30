use super::*;
use crate::{
    application::test_support::with_application,
    editor::{
        presentation::tests::{key, pointer},
        tests::{add_ready_fixture, app_frame, fixture, with_document_services},
    },
};

#[test]
fn escape_targets_the_replacement_not_hidden_region_work() {
    for measuring in [false, true] {
        let mut editor = with_document_services(fixture());
        add_ready_fixture(&mut editor);
        editor.sync_document();
        let (reply, receive) = futures_channel::oneshot::channel();
        editor.document.spatial_reply = Some(receive);
        let metadata = editor.document.capture().ready_spatial_metadata();
        let hold = crate::background::testing::pause();
        with_application(editor, |app| {
            app.enter(|editor, cx| editor.request_analysis(cx));
            app.pump();
            app.enter(|editor, cx| {
                if measuring {
                    editor.begin_pick(super::super::Pick::Dmin, cx);
                } else {
                    editor.begin_crop();
                }
            });
            app.pump();
            if measuring {
                app.update(|editor| {
                    let area = editor.document.visit.canvas.ready.as_ref().unwrap().area;
                    editor.set_pick_region(Some(crate::coverage::Coverage::rectangle(
                        crate::raw::Rect {
                            width: 16,
                            height: 16,
                            ..area
                        },
                    )));
                    assert!(editor.submit_picker());
                });
                app.pump();
            }
            let context = egui::Context::default();
            app_frame(app, &context, vec![]);
            app_frame(app, &context, key(egui::Key::Escape));
            app.read(|editor| {
                assert!(
                    editor.document.spatial_preparing.is_some(),
                    "hidden useful work must not consume Escape"
                );
                if measuring {
                    assert!(
                        editor.busy && editor.measurement_pending(),
                        "hidden work cannot unblock a running measurement"
                    );
                } else {
                    assert!(
                        editor.document.crop_edit().is_none(),
                        "Escape must reach the visible Crop draft"
                    );
                }
            });
            // Retiring hidden work from another lifecycle path must also preserve
            // the replacement's foreground activity and busy state.
            app.update(Editor::cancel_spatial);
            if measuring {
                app.read(|editor| assert!(editor.busy && editor.measurement_pending()));
            }
            drop(hold);
            app.run_until(std::time::Duration::from_secs(10), |editor| !editor.busy);
            let _ = reply.send(Ok(Some(metadata)));
        });
    }
}

#[test]
fn running_regions_sampling_and_fitting_leave_tool_switching_reachable() {
    for phase in 0..3 {
        let mut editor = with_document_services(fixture());
        add_ready_fixture(&mut editor);
        editor.sync_document();
        let before = editor.document.state.project.clone();
        let admission = crate::background::testing::pause();
        with_application(editor, |app| {
            app.enter(|editor, cx| {
                match phase {
                    0 => editor.request_analysis(cx),
                    1 => {
                        let request = editor.analysis_request().unwrap();
                        editor.prepare_analysis_review(request, cx);
                    }
                    _ => editor.begin_analysis(cx),
                }
                editor.tasks.elapse(Duration::from_secs(1));
            });
            app.pump();
            let context = egui::Context::default();
            app_frame(app, &context, vec![]);
            let labels = app_frame(app, &context, vec![]);
            assert!(
                context
                    .memory(|memory| memory.area_rect(egui::Id::new("working-document")))
                    .is_none()
            );
            let tab = labels
                .iter()
                .find(|(text, _)| text == "Photo")
                .unwrap()
                .1
                .center();
            for pressed in [true, false] {
                app_frame(app, &context, pointer(tab, pressed));
            }
            assert_eq!(app.read(|editor| editor.tab), 1);
            let labels = app_frame(app, &context, vec![]);
            let crop = labels
                .iter()
                .find(|(text, _)| text == "Crop & straighten")
                .unwrap()
                .1
                .center();
            for pressed in [true, false] {
                app_frame(app, &context, pointer(crop, pressed));
            }
            app.read(|editor| {
                assert!(
                    editor.document.crop_edit().is_some(),
                    "phase {phase}: {}",
                    editor.status
                );
                assert!(editor.document.analysis_operation().is_none());
                assert!(!editor.busy);
                assert_eq!(editor.document.project, before);
            });
            drop(admission);
            app.run_until(Duration::from_secs(10), |editor| {
                editor.document.spatial_preparing.is_none()
            });
            app.read(|editor| {
                assert!(editor.document.crop_edit().is_some());
                assert!(editor.tasks.view().is_none());
                assert_eq!(editor.document.project, before);
            });
        });
    }
}
