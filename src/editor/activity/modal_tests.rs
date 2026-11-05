use super::*;
use crate::{
    application::test_support::with_application,
    editor::{
        Editor,
        presentation::tests::pointer,
        tests::{add_ready_fixture, app_frame, fixture, with_document_services},
    },
};

#[test]
fn slow_measurement_exposes_modal_cancel_without_accepting_a_late_worker_result() {
    let mut editor = with_document_services(fixture());
    add_ready_fixture(&mut editor);
    editor.sync_document();
    let before = editor.document.project.clone();
    let gate = crate::editor::measurement::testing::Gate::install(&mut editor);
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.begin_test_picker(crate::editor::PickerCase::Dmin, cx);
        });
        app.pump();
        app.update(|editor| {
            editor.set_test_pick_region(Some(crate::coverage::Coverage::rectangle(
                crate::raw::Rect {
                    x: 40,
                    y: 40,
                    width: 16,
                    height: 16,
                },
            )));
            assert!(editor.submit_test_picker());
            assert!(!editor.picking());
        });
        let worker = gate.next(app);
        app.update(|editor| {
            assert!(!editor.has_hosted_tool());
            editor.activities.elapse(Duration::from_secs(1));
        });

        let context = egui::Context::default();
        app_frame(app, &context, vec![]);
        let labels = app_frame(app, &context, vec![]);
        let cancel = labels
            .iter()
            .find(|(label, _)| label == "Cancel measurement")
            .unwrap()
            .1
            .center();
        for pressed in [true, false] {
            app_frame(app, &context, pointer(cancel, pressed));
        }
        app.run_until(Duration::from_secs(10), |editor| {
            !editor.measurement_pending()
        });
        assert!(worker.cancelled.is_cancelled());
        app.read(|editor| {
            assert!(editor.activities.view().is_none());
            assert_eq!(editor.document.project, before);
        });
        worker.run();
        app.pump();
        app.read(|editor| assert_eq!(editor.document.project, before));
    });
}

#[test]
fn regions_preparation_owns_modal_feedback_even_after_its_requesting_tool_retires() {
    let mut editor = with_document_services(fixture());
    add_ready_fixture(&mut editor);
    editor.sync_document();
    let (reply, receive) = futures_channel::oneshot::channel();
    editor.document.spatial_reply = Some(receive);
    let metadata = editor.document.capture().ready_spatial_metadata();
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.request_analysis(cx));
        app.pump();
        app.update(Editor::end_tool);
        app.pump();
        app.update(|editor| {
            assert!(editor.document.spatial_preparing.is_some());
            assert!(editor.activities.blocks_workspace());
            assert!(editor.interaction_blocked());
            editor.begin_crop();
            assert!(
                !editor.cropping(),
                "the still-running producer owns the modal interaction"
            );
        });
        reply.send(Ok(Some(metadata))).unwrap();
        app.run_until(Duration::from_secs(10), |editor| {
            editor.document.spatial_preparing.is_none()
        });
        app.update(|editor| {
            assert!(!editor.interaction_blocked());
            editor.begin_crop();
            assert!(
                editor.cropping(),
                "completion restores normal tool admission"
            );
        });
    });
}

#[test]
fn regions_sampling_and_fitting_block_unrelated_controls_but_expose_cancellation() {
    for phase in 0..3 {
        let mut editor = with_document_services(fixture());
        add_ready_fixture(&mut editor);
        editor.sync_document();
        let before = editor.document.project.clone();
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
                editor.activities.elapse(Duration::from_secs(1));
            });
            app.pump();
            let context = egui::Context::default();
            app_frame(app, &context, vec![]);
            let labels = app_frame(app, &context, vec![]);
            assert!(
                context
                    .memory(|memory| memory.area_rect(egui::Id::new("working-document")))
                    .is_some()
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
            app.read(|editor| {
                assert_eq!(editor.tab, 0);
                assert!(editor.interaction_blocked());
                assert!(!editor.tool_start_available());
                assert_eq!(editor.document.project, before);
            });
            let labels = app_frame(app, &context, vec![]);
            let label = if phase == 0 {
                "Cancel region preparation"
            } else {
                "Cancel analysis"
            };
            let cancel = labels
                .iter()
                .find(|(text, _)| text == label)
                .unwrap()
                .1
                .center();
            for pressed in [true, false] {
                app_frame(app, &context, pointer(cancel, pressed));
            }
            drop(admission);
            app.run_until(Duration::from_secs(10), |editor| {
                !editor.interaction_blocked()
            });
            app.read(|editor| assert_eq!(editor.document.project, before));
        });
    }
}
