use super::*;
use crate::{
    application::test_support::with_application,
    editor::{
        presentation::tests::pointer,
        tests::{add_ready_fixture, app_frame, fixture, with_document_services},
    },
};

#[test]
fn running_regions_sampling_and_fitting_are_modal_cancellable_and_restore_the_workspace() {
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
            let modal = context.memory(|memory| memory.area_rect(egui::Id::new("working-document")))
                .expect("blocked analysis work must have an actual shared modal, not just status-bar feedback");
            let label = if phase == 0 {
                "Cancel region preparation"
            } else {
                "Cancel analysis"
            };
            let actions: Vec<_> = labels.iter().filter(|(text, _)| text == label).collect();
            assert_eq!(
                actions.len(),
                1,
                "one usable cancellation owner, not modal plus status-bar duplicates"
            );
            let cancel = actions[0].1.center();
            assert!(
                modal.contains(cancel),
                "Cancel must be reachable inside the modal"
            );
            let tab = labels
                .iter()
                .find(|(text, _)| text == "Photo")
                .unwrap()
                .1
                .center();
            let previous = app.read(|editor| editor.tab);
            for pressed in [true, false] {
                app_frame(app, &context, pointer(tab, pressed));
            }
            assert_eq!(
                app.read(|editor| editor.tab),
                previous,
                "the modal must actually intercept background interaction"
            );
            for pressed in [true, false] {
                app_frame(app, &context, pointer(cancel, pressed));
            }
            drop(admission);
            app.run_until(Duration::from_secs(5), |editor| !editor.busy);
            app.read(|editor| {
                assert!(editor.tasks.view().is_none());
                assert!(editor.document.spatial_preparing.is_none());
                assert!(editor.document.analysis_preparing.is_none());
                assert!(editor.document.analysis_pending.is_none());
                assert_eq!(editor.document.state.project, before);
            });
            app_frame(app, &context, vec![]);
            let labels = app_frame(app, &context, vec![]);
            let tab = labels
                .iter()
                .find(|(text, _)| text == "Photo")
                .unwrap()
                .1
                .center();
            for pressed in [true, false] {
                app_frame(app, &context, pointer(tab, pressed));
            }
            assert_eq!(
                app.read(|editor| editor.tab),
                1,
                "completion must release modal input capture"
            );
        });
    }
}
