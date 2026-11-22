use super::*;

#[test]
fn measurement_pickers_keep_the_selected_look_without_reframing_the_photo() {
    fixture_editor(|mut h, _| {
        let asset = constant_asset(0.4);
        let label = asset.name().to_owned();
        let id = asset.id();
        h.update(|editor| editor.look.loaded.push(asset));
        choose(&mut h, &label);
        paint_until(&mut h, positive_ready);
        for (size, width) in [
            (egui::vec2(1100., 800.), 315.),
            (egui::vec2(800., 600.), 270.),
        ] {
            set_layout(&mut h, size, width);
            for kind in [
                PickerCase::Dmin,
                PickerCase::Dmax,
                PickerCase::Gamma,
                PickerCase::RollWb,
                PickerCase::Wb,
            ] {
                stage(
                    &mut h,
                    if kind == PickerCase::Wb {
                        "Photo"
                    } else {
                        "Roll"
                    },
                );
                paint_until(&mut h, |editor| {
                    let working = &editor.document.visit.working_view;
                    working.idle()
                        && positive_ready(editor)
                        && editor
                            .canvas_frame
                            .displayed
                            .as_ref()
                            .is_some_and(|painted| {
                                working
                                    .displayed
                                    .as_ref()
                                    .is_some_and(|view| Arc::ptr_eq(view, &painted.view))
                            })
                });
                let (view, composition, viewport, revision) = h.read(|editor| {
                    (
                        editor.canvas_frame.displayed.as_ref().unwrap().view.clone(),
                        editor.active_canvas().last_layout.unwrap().0.composition,
                        editor.document.visit.canvas.viewport,
                        editor.document.visit.working_view.revision(),
                    )
                });
                picking::open(&mut h, kind);
                h.render().unwrap();
                h.read(|editor| {
                    assert_eq!(editor.test_pick(), Some(kind.display()));
                    assert!(!editor.document.source);
                    assert_eq!(
                        editor
                            .canvas_frame
                            .displayed
                            .as_ref()
                            .unwrap()
                            .view
                            .look_id(),
                        Some(id)
                    );
                    assert!(Arc::ptr_eq(
                        &editor.canvas_frame.displayed.as_ref().unwrap().view,
                        &view
                    ));
                    assert_eq!(editor.document.visit.working_view.revision(), revision);
                    assert_eq!(
                        editor.active_canvas().last_layout.unwrap().0.composition,
                        composition
                    );
                    assert_eq!(editor.document.visit.canvas.viewport, viewport);
                });
                h.get_by_role_and_label(egui::accesskit::Role::Button, "Cancel")
                    .click();
                h.step();
            }
        }
    });
}
