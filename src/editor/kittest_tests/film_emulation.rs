use super::workflow::enter_numeric;
use super::*;

fn painted(harness: &mut Harness<'_, Editor>) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        harness.step();
        harness.render().unwrap();
        let ready = harness.read(|editor| {
            editor.working.displayed.as_ref().is_some_and(|view| {
                view.processing
                    == editor
                        .positive_processing(
                            editor.input,
                            editor.roll,
                            editor.adjustment_layer_settings,
                        )
                        .unwrap()
                        .signature()
                    && !view.in_flight()
            })
        });
        if ready {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "film working view did not settle"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn film_controls_persist_render_and_reset_without_changing_target_at_both_display_scales() {
    editor_harness_with_gpu(true, |mut h, _, _, _| {
        h.enter(|editor, cx| editor.add_raws(vec![fixture("roll-0.dng")], cx));
        wait_for(&mut h, |editor| {
            !editor.busy && editor.document.visit.ready.is_some()
        });
        stage(&mut h, "Rendering");
        assert!(
            h.query_by_label("Film Emulation").is_none(),
            "visibility is Digital-only"
        );
        h.enter(|editor, cx| editor.apply_input(crate::processing::InputSettings::Digital, cx));
        wait_for(&mut h, |editor| {
            !editor.busy && editor.input == crate::processing::InputSettings::Digital
        });
        h.update(|editor| {
            editor.source = false;
            editor.scopes.visible = true;
            editor.target = OutputTarget::Srgb;
            editor.sync_document();
        });
        for scale in [1., 2.] {
            h.input_mut()
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .native_pixels_per_point = Some(scale);
            stage(&mut h, "Rendering");
            h.run();
            let enabled = h.read(|editor| editor.film_emulation.enabled);
            if !enabled {
                h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Enable")
                    .scroll_to_me();
                h.try_run_realtime().unwrap();
                h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Enable")
                    .click();
                h.step();
            }
            h.get_by_role_and_label(egui::accesskit::Role::Button, "Coarse")
                .click();
            h.step();
            h.read(|editor| {
                assert_eq!(
                    editor.film_emulation,
                    crate::film_emulation::FilmEmulationPreset::Coarse.settings()
                )
            });
            enter_numeric(&mut h, "Size", "0.090");
            h.try_run_realtime().unwrap();
            enter_numeric(&mut h, "Amount", "0.071");
            h.try_run_realtime().unwrap();
            painted(&mut h);
            h.read(|editor| {
                assert!(
                    editor.document.project.output.film_emulation.enabled,
                    "{}",
                    editor.status
                );
                assert_eq!(editor.document.project.output.film_emulation.amount, 0.071);
                assert_eq!(editor.document.project.output.film_emulation.size, 0.090);
                assert!(editor.document.dirty());
                let serialized = serde_json::to_string(&editor.document.project).unwrap();
                let reopened: crate::project::Project = serde_json::from_str(&serialized).unwrap();
                assert_eq!(reopened.output.film_emulation, editor.film_emulation);
                let source = editor.current_source.unwrap();
                assert_eq!(
                    editor
                        .positive_processing(
                            editor.input,
                            editor.roll,
                            editor.adjustment_layer_settings
                        )
                        .unwrap()
                        .film_seed(),
                    Some(source.0)
                );
            });
            let label = h.get_by_role_and_label(egui::accesskit::Role::Label, "Amount");
            label.scroll_to_me();
            h.try_run_realtime().unwrap();
            let point = h
                .get_by_role_and_label(egui::accesskit::Role::Label, "Amount")
                .rect()
                .center();
            h.event(egui::Event::ModifiersChanged(egui::Modifiers::ALT));
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::ALT,
                });
            }
            h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
            h.step();
            h.try_run_realtime().unwrap();
            h.read(|editor| {
                assert_eq!(
                    editor.film_emulation.amount,
                    crate::film_emulation::DEFAULT_AMOUNT
                );
                assert_eq!(
                    editor.target,
                    OutputTarget::Srgb,
                    "film reset must not reset the output target"
                );
            });
        }
    });
}
