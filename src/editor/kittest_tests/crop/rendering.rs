use super::*;

#[test]
fn crop_replaces_temporary_proxy_with_requested_rendering() {
    proxy_then_rendering(false);
}

#[test]
fn crop_with_repairs_keeps_proxy_visible_until_requested_rendering_arrives() {
    proxy_then_rendering(true);
}

fn proxy_then_rendering(repaired: bool) {
    editor_harness(|mut h, _| {
        h.update(|editor| editor.document.project.input = InputSettings::Digital);
        h.enter(|editor, cx| editor.add_raws(vec![fixture("patches.dng")], cx));
        wait_for(&mut h, |editor| {
            !editor.interaction_blocked() && editor.document.visit.working_view.idle()
        });
        stage(&mut h, "Photo");
        let button = h.get_by_role_and_label(egui::accesskit::Role::Button, "Crop & straighten");
        button.scroll_to_me();
        h.try_run_realtime().unwrap();
        let acquisition = crate::background::testing::pause();
        h.update(|editor| {
            if repaired {
                let id = editor.document.visit.current_source.unwrap();
                editor
                    .document
                    .project
                    .source_mut(id)
                    .unwrap()
                    .repairs
                    .insert(
                        crate::dust_repair::SupportMask::new([90, 30], [2, 2], vec![true; 4])
                            .unwrap(),
                        crate::dust_repair::RepairOrigin::Manual,
                        [-39, 0],
                    )
                    .unwrap();
            }
            editor.document.visit.working_view.clear();
        });
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Crop & straighten")
            .click();
        h.step();
        h.read(|editor| {
            assert!(editor.cropping());
            assert!(
                editor.canvas_frame.displayed.is_some(),
                "paint temporary feedback"
            );
            assert!(
                editor.active_canvas().preview.rendered().is_some(),
                "use the available camera proxy even with repairs"
            );
            let working = &editor.document.visit.working_view;
            assert!(working.displayed.is_none());
            assert!(
                working.task.is_some(),
                "Crop requests rendering without a repair-specific exception"
            );
            assert!(!working.support_ready());
        });
        save_capture(
            &h.render().unwrap(),
            if repaired {
                "crop-render-repairs-proxy"
            } else {
                "crop-render-clean-proxy"
            },
        );
        drop(acquisition);
        wait_for(&mut h, |editor| {
            editor.document.visit.working_view.idle()
                && editor.active_canvas().preview.demand().is_none()
                && editor
                    .canvas_frame
                    .displayed
                    .as_ref()
                    .is_some_and(|painted| {
                        editor
                            .document
                            .visit
                            .working_view
                            .displayed
                            .as_ref()
                            .is_some_and(|view| Arc::ptr_eq(view, &painted.view))
                    })
        });
        save_capture(
            &h.render().unwrap(),
            if repaired {
                "crop-render-repairs-completed"
            } else {
                "crop-render-clean-completed"
            },
        );
        h.read(|editor| {
            let view = &editor.canvas_frame.displayed.as_ref().unwrap().view;
            assert_eq!(
                view.capture.recipe().unwrap().repairs.len(),
                usize::from(repaired)
            );
            let ready = editor.active_canvas().ready.as_ref().unwrap();
            assert_eq!(
                view.spec.frame().composition,
                Composition::new(ready.area, Default::default(), Default::default()).unwrap(),
                "render the full native-axis source beneath Crop's draft"
            );
        });
        h.update(|editor| {
            editor
                .document
                .visit
                .working_view
                .acquisition_failed(Error::Gpu("Fixture rendering failure".into()))
        });
        h.run_steps(2);
        let error =
            h.get_by_label("Preview not updated — showing previous image: calculation failed");
        h.read(|editor| {
            assert!(error.rect().top() >= editor.canvas_frame.layout.unwrap().0.canvas.bottom());
            assert_eq!(
                editor.document.visit.working_view.error,
                Some(Error::Gpu("Fixture rendering failure".into()))
            );
        });
        save_capture(
            &h.render().unwrap(),
            if repaired {
                "crop-render-repairs-error"
            } else {
                "crop-render-clean-error"
            },
        );
    });
}
