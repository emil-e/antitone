use super::*;
use crate::gpu::test_inputs::AuthoredPhotograph;

#[test]
fn standalone_reference_supplies_the_painted_canvas_and_scopes_at_both_layouts() {
    editor_harness(|mut harness, _| {
        harness.update(|editor| {
            assert!(editor.document.visit.canvas.ready.is_none());
            assert!(editor.document.project.order.is_empty());
            editor.document.project.orientation.turn_clockwise();

            editor.scopes.visible = true;
            editor.preferences.scopes_visible = true;
        });
        let before = harness.read(|editor| editor.document.project.clone());
        harness.enter(|editor, cx| editor.begin_flat_edit(fixture("bare-light.dng"), None, cx));
        wait_for(&mut harness, |editor| !editor.interaction_blocked());
        for (name, size, width) in [
            ("normal", egui::vec2(1100., 800.), 315.),
            ("narrow", egui::vec2(800., 600.), 270.),
        ] {
            set_layout(&mut harness, size, width);
            workflow::wait_settled(&mut harness);
            harness.read(|editor| {
                assert!(
                    editor.document.visit.canvas.ready.is_none(),
                    "reference is not installed into the photograph"
                );
                let state = editor.document.flat_edit().unwrap();
                let reference = state.view.ready.as_ref().unwrap();
                let layout = editor.canvas_frame.layout.unwrap().0;
                assert_eq!(
                    layout.composition,
                    Composition::new(reference.area, Default::default(), Default::default())
                        .unwrap()
                );
                let painted = editor
                    .canvas_frame
                    .displayed
                    .as_ref()
                    .expect("reference paint product");
                let scopes::ScopeInput {
                    view,
                    mapping,
                    frame,
                } = editor.scope_request().unwrap();
                assert!(Arc::ptr_eq(&view, &painted.view));
                assert_eq!(mapping, painted.mapping);
                assert!(frame.output.into_iter().all(|edge| edge <= 1024));
                let expected = editor
                    .source_preview_parameters(
                        reference
                            .developed
                            .metadata()
                            .source_preview(reference.developed.white_balance()),
                        editor.source_exposure,
                    )
                    .unwrap();
                assert_eq!(view.processing(), AuthoredPhotograph::from(expected));
                assert!(view.spec.is_inspection());
                assert_eq!(editor.document.project, before);
            });
            let timing =
                harness.read(|editor| Arc::new(SelectionTiming::new(editor.id, Instant::now())));
            harness.update(|editor| {
                editor.document.flat_edit_mut().unwrap().view.paint_observer = Some(timing.clone())
            });
            harness.step();
            let image = harness.render().unwrap();
            assert!(
                timing.ready_was_painted(),
                "the reference GPU callback must run"
            );
            assert_eq!(image.dimensions(), (size.x as u32, size.y as u32));
            save_capture(&image, &format!("flat-standalone-{name}-offscreen"));
        }
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Cancel")
            .click();
        harness.step();
        harness.read(|editor| {
            assert!(!editor.has_hosted_tool());
            assert!(editor.canvas_frame.displayed.is_none());
            assert!(editor.scope_request().is_err());
            assert_eq!(editor.document.project, before);
        });
    });
}
