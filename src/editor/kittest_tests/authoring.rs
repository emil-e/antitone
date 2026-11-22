use super::workflow::enter_numeric;
use super::*;

#[test]
fn controls_and_layers_remain_authorable_until_calibration_can_render() {
    tools::calibrated_harness(|mut h| {
        let input = h.read(|editor| editor.input());
        for (size, width, missing, suffix) in [
            (egui::vec2(1100., 800.), 315., true, "missing-normal"),
            (egui::vec2(800., 600.), 270., false, "incompatible-narrow"),
        ] {
            set_layout(&mut h, size, width);
            h.update(|editor| {
                editor.document.project.input = input;
                if let InputSettings::Negative { dmin, dmax, .. } =
                    &mut editor.document.project.input
                {
                    *dmax = if missing { None } else { *dmin };
                }
                editor.document.source = missing;
            });
            stage(&mut h, "Roll");
            if h.query_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
                .is_none()
            {
                h.get_by_role_and_label(egui::accesskit::Role::Button, "Exposure")
                    .scroll_to_me();
                h.try_run_realtime().unwrap();
                h.get_by_role_and_label(egui::accesskit::Role::Button, "Exposure")
                    .click();
                h.step();
            }
            let roll_exposure = if missing { 1.25 } else { 1.5 };
            enter_numeric(&mut h, "Exposure", &roll_exposure.to_string());
            enter_numeric(&mut h, "Gamma", "0.9");
            wait_for(&mut h, |e| !e.document.gamma_pivot.calculating());
            h.read(|editor| {
                assert_eq!(
                    editor.document.project.roll.exposure, roll_exposure,
                    "{}",
                    editor.status
                );
                assert_eq!(editor.input().gamma(), 0.9, "{}", editor.status);
            });
            adjustment_layers::show_layers(&mut h);
            let count = h.read(|editor| adjustment_layers::layers(editor).layers.len());
            let add = h.get_by_label("Add global");
            assert!(!add.accesskit_node().is_disabled());
            adjustment_layers::click(&mut h, "Add global");
            assert_eq!(
                h.read(|editor| adjustment_layers::layers(editor).layers.len()),
                count + 1
            );
            enter_numeric(&mut h, "Exposure", "0.75");
            h.read(|editor| {
                assert_eq!(editor.photo_settings().exposure, 0.75, "{}", editor.status)
            });
            if !missing {
                h.get_by_label_contains("Positive preview unavailable:");
                adjustment_layers::show_layers(&mut h);
                adjustment_layers::click(&mut h, "Add linear");
                let image = h.read(|editor| editor.active_canvas().last_layout.unwrap().0.image);
                masks::pointer(&mut h, image.center() - egui::vec2(30., 20.), true);
                masks::pointer(&mut h, image.center() + egui::vec2(30., 20.), false);
                h.read(|editor| {
                    assert!(
                        editor.masking() && !editor.creating_mask() && !editor.mask_dragging(),
                        "{}",
                        editor.status
                    );
                    assert_eq!(adjustment_layers::layers(editor).layers.len(), count + 2);
                });
            }
            h.try_run_realtime().unwrap();
            save_capture(&h.render().unwrap(), &format!("control-authoring-{suffix}"));
            h.enter(|editor, _| {
                crate::editor::calibration::set_dmax(editor, input.calibration().unwrap().dmax)
                    .unwrap();
            });
            if h.read(|editor| editor.document.source) {
                h.get_by_label("View source").click();
                h.step();
            }
            workflow::wait_settled(&mut h);
            h.read(|editor| {
                assert_eq!(editor.document.project.roll.exposure, roll_exposure);
                assert_eq!(editor.input().gamma(), 0.9);
                assert!(!editor.document.source);
            });
            assert!(
                h.query_by_label_contains("Positive preview unavailable:")
                    .is_none()
            );
            save_capture(
                &h.render().unwrap(),
                &format!("control-authoring-recovered-{suffix}"),
            );
        }
    });
}
