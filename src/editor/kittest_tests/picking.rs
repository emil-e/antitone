use super::*;

fn open(h: &mut Harness<'_, Editor>, kind: Pick) {
    let (tab, label) = match kind {
        Pick::Dmin => ("Roll", "Pick Dmin (film base)"),
        Pick::Dmax => ("Roll", "Pick Dmax"),
        Pick::Gamma => ("Roll", "Pick neutral for gamma balance"),
        Pick::RollWb => ("Roll", "Pick neutral for white balance"),
        Pick::Wb => ("Photo", "Pick neutral for white balance"),
        Pick::Analysis => unreachable!("legacy area selection is not a measurement mode"),
    };
    stage(h, tab);
    h.get_by_label(label).scroll_to_me();
    h.try_run_realtime().unwrap();
    h.get_by_label(label).click();
    h.step();
    h.try_run_realtime().unwrap();
}

#[test]
fn measurement_modes_own_apply_readiness_and_leave_normal_view_preferences_intact() {
    workflow::calibrated_workflow(|mut h, _, _directory| {
        for (size, width, suffix) in [
            (egui::vec2(1100., 800.), 315., "normal"),
            (egui::vec2(800., 600.), 270., "narrow"),
        ] {
            set_layout(&mut h, size, width);
            for kind in [Pick::Dmin, Pick::Dmax, Pick::Gamma, Pick::RollWb, Pick::Wb] {
                let source_preference = matches!(kind, Pick::Gamma | Pick::RollWb | Pick::Wb);
                h.update(|editor| editor.document.source = source_preference);
                h.step();
                let viewport = h.read(|editor| editor.document.visit.canvas.viewport);
                let before = h.read(|editor| editor.document.state.project.clone());
                open(&mut h, kind);
                h.read(|editor| {
                    assert!(
                        editor
                            .current_tool()
                            .unwrap()
                            .is::<super::super::picking::PickTool>()
                    );
                    assert_eq!(editor.active_pick(), Some(kind.display()));
                    assert_eq!(
                        editor.document.source, source_preference,
                        "mode does not commandeer the normal source preference"
                    );
                    assert_eq!(editor.canvas_inspection(), !source_preference);
                });
                let apply = h.get_by_role_and_label(egui::accesskit::Role::Button, "Apply");
                assert!(apply.accesskit_node().is_disabled());
                h.get_by_label(&format!("Active tool: {}", kind.display().label));
                let point =
                    h.read(|editor| editor.active_canvas().last_layout.unwrap().0.image.center());
                h.hover_at(point);
                for pressed in [true, false] {
                    h.event(egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    });
                    h.step();
                }
                h.step();
                assert!(
                    !h.get_by_role_and_label(egui::accesskit::Role::Button, "Apply")
                        .accesskit_node()
                        .is_disabled()
                );
                h.read(|editor| {
                    assert!(editor.pick_region().is_some());
                    assert_eq!(editor.document.state.project, before);
                });
                h.event(egui::Event::PointerGone);
                h.step();
                save_capture(
                    &h.render().unwrap(),
                    &format!(
                        "measurement-{}-{suffix}",
                        match kind {
                            Pick::Dmin => "dmin",
                            Pick::Dmax => "dmax",
                            Pick::Gamma => "gamma",
                            Pick::RollWb => "roll-wb",
                            Pick::Wb => "layer-wb",
                            Pick::Analysis => unreachable!(),
                        }
                    ),
                );
                h.get_by_role_and_label(egui::accesskit::Role::Button, "Cancel")
                    .click();
                h.step();
                h.try_run_realtime().unwrap();
                h.read(|editor| {
                    assert!(editor.current_tool().is_none());
                    assert!(editor.active_pick().is_none());
                    assert_eq!(editor.document.source, source_preference);
                    assert_eq!(editor.canvas_inspection(), source_preference);
                    assert_eq!(editor.document.visit.canvas.viewport, viewport);
                    assert_eq!(editor.document.state.project, before);
                });
            }
        }
    });
}
