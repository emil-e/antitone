use super::workflow::enter_numeric;
use super::*;

pub(super) fn painted(harness: &mut Harness<'_, Editor>) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        harness.step();
        harness.render().unwrap();
        let ready = harness.read(|editor| {
            editor
                .document
                .visit
                .working_view
                .displayed
                .as_ref()
                .is_some_and(|view| {
                    view.processing
                        == editor
                            .positive_processing(
                                editor.document.input,
                                editor.document.roll,
                                editor.document.visit.adjustment_layer_settings,
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

fn select_preset(h: &mut Harness<'_, Editor>, label: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Film preset")
        .scroll_to_me();
    h.try_run_realtime().unwrap();
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Film preset")
        .click();
    h.run();
    h.get_by_label(label).click();
    h.step();
    h.try_run_realtime().unwrap();
}

#[test]
fn fixed_scale_presets_preserve_enablement_edits_provenance_and_history_at_both_display_scales() {
    for scale in [1., 2.] {
        editor_harness_with_gpu(true, |mut h, _, _, _| {
            h.enter(|e, cx| e.add_raws(vec![fixture("roll-0.dng")], cx));
            wait_for(&mut h, |e| {
                !e.busy && e.document.visit.canvas.ready.is_some()
            });
            stage(&mut h, "Rendering");
            assert!(
                h.query_by_label("Film texture").is_none(),
                "visibility is Digital-only"
            );
            h.enter(|e, cx| e.apply_input(crate::processing::InputSettings::Digital, cx));
            wait_for(&mut h, |e| {
                !e.busy && e.document.input == crate::processing::InputSettings::Digital
            });
            let directory = tempfile::tempdir().unwrap();
            h.input_mut()
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .native_pixels_per_point = Some(scale);
            h.update(|e| {
                e.document.source = false;
                e.document.target = OutputTarget::Srgb;
                e.document.film_emulation = FilmEmulationSettings::default();
                e.document.halation = crate::halation::HalationSettings {
                    enabled: true,
                    ..Default::default()
                };
                e.document.project.output.film_preset = FilmPreset::ThirtyFiveMm;
                e.sync_document();
            });
            stage(&mut h, "Rendering");
            h.run();
            let before_collapse = h.read(|e| e.document.project.clone());
            h.get_by_label("Film").scroll_to_me();
            h.try_run_realtime().unwrap();
            h.get_by_label("Film").click();
            h.run();
            assert!(h.query_by_label("Film preset").is_none());
            assert!(h.query_by_label("Film texture").is_none());
            assert!(h.query_by_label("Enable halation").is_none());
            assert!(
                h.query_by_role_and_label(egui::accesskit::Role::ComboBox, "Look")
                    .is_some(),
                "Film grouping must not encompass the creative look"
            );
            assert!(
                h.query_by_role_and_label(egui::accesskit::Role::ComboBox, "Target")
                    .is_some(),
                "output target remains outside Film"
            );
            h.read(|e| {
                assert_eq!(
                    e.document.project, before_collapse,
                    "collapsing Film must not bypass or reset its effects"
                )
            });
            h.get_by_label("Film").click();
            h.run();
            select_preset(&mut h, "Medium format");
            h.read(|e| {
                assert!(
                    !e.document.film_emulation.enabled,
                    "choosing a format is not a master enable"
                );
                assert!(e.document.halation.enabled);
                assert!((e.document.film_emulation.size - 0.017142857142857144).abs() < 1e-14);
                assert!(
                    (e.document.film_emulation.image_sigma - 0.017142857142857144).abs() < 1e-14
                );
                assert!((e.document.halation.size - 0.046714285714285715).abs() < 1e-14);
                assert_eq!(e.document.film_emulation.amount, 0.065);
                assert_eq!(e.document.halation.strength, 0.54);
            });
            h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Enable")
                .scroll_to_me();
            h.try_run_realtime().unwrap();
            h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Enable")
                .click();
            h.step();
            h.update(|e| {
                e.document.film_emulation.image_sigma *= 0.5;
                e.sync_document();
            });
            enter_numeric(&mut h, "Size", "0.090");
            h.try_run_realtime().unwrap();
            enter_numeric(&mut h, "Amount", "0.071");
            h.try_run_realtime().unwrap();
            painted(&mut h);
            h.read(|e| {
                assert_eq!(
                    e.document.film_emulation.size, 0.090,
                    "fixed units, not a format multiplier"
                );
                assert!((e.document.film_emulation.image_sigma - 0.045).abs() < 1e-14);
                assert_eq!(e.document.film_emulation.amount, 0.071);
                assert_eq!(
                    e.document.project.output.film_preset,
                    FilmPreset::MediumFormat
                );
                assert!(
                    !FilmPreset::MediumFormat
                        .matches(e.document.film_emulation, e.document.halation)
                );
                assert!(e.document.dirty());
                let reopened: crate::project::Project = serde_json::from_str(
                    &serde_json::to_string(&e.document.state.project).unwrap(),
                )
                .unwrap();
                assert_eq!(reopened.output.film_preset, FilmPreset::MediumFormat);
                assert_eq!(reopened.output.film_emulation, e.document.film_emulation);
                assert_eq!(reopened.output.halation, e.document.halation);
                assert_eq!(e.document.target, OutputTarget::Srgb);
            });
            if scale == 1. {
                save_capture(
                    &h.render().unwrap(),
                    "film-medium-format-modified-offscreen",
                );
            }
            assert_eq!(
                h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Film preset")
                    .accesskit_node()
                    .value()
                    .as_deref(),
                Some("Medium format · modified")
            );
            let saved_values = h.read(|e| (e.document.film_emulation, e.document.halation));
            let path = directory.path().join(format!("film-{scale}.antitone"));
            h.update(|e| e.document.path = Some(path.clone()));
            native_file_action(&mut h, crate::platform::menu::Action::Save);
            wait_for(&mut h, |e| !e.busy && e.document_pending.is_none());
            h.read(|e| assert!(!e.document.dirty(), "{}", e.status));
            let saved = crate::project::Project::load(&path).unwrap();
            assert_eq!(saved.output.film_preset, FilmPreset::MediumFormat);
            assert_eq!(
                (saved.output.film_emulation, saved.output.halation),
                saved_values
            );
            native_file_action(&mut h, crate::platform::menu::Action::New);
            wait_for(&mut h, |e| !e.busy && e.document_pending.is_none());
            open_project(&mut h, path);
            wait_for(&mut h, |e| {
                !e.busy && e.document_pending.is_none() && e.document.visit.canvas.ready.is_some()
            });
            stage(&mut h, "Rendering");
            h.run();
            h.read(|e| {
                assert_eq!(
                    (e.document.film_emulation, e.document.halation),
                    saved_values
                );
                assert_eq!(
                    e.document.project.output.film_preset,
                    FilmPreset::MediumFormat
                );
            });
            assert_eq!(
                h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Film preset")
                    .accesskit_node()
                    .value()
                    .as_deref(),
                Some("Medium format · modified")
            );
            let label = h.get_by_role_and_label(egui::accesskit::Role::Label, "Size");
            label.scroll_to_me();
            h.try_run_realtime().unwrap();
            let point = h
                .get_by_role_and_label(egui::accesskit::Role::Label, "Size")
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
            h.read(|e| {
                assert_eq!(e.document.film_emulation.size, 0.040);
                assert!(
                    (e.document.film_emulation.image_sigma - 0.020).abs() < 1e-14,
                    "reset Size preserves the saved relationship rather than guessing a preset"
                );
                assert_eq!(e.document.halation, saved_values.1);
            });
            let previous = h.read(|e| (e.document.film_emulation, e.document.halation));
            select_preset(&mut h, "35mm");
            h.enter(|e, cx| e.restore_history(super::super::history::Direction::Undo, cx));
            wait_for(&mut h, |e| !e.busy);
            h.read(|e| {
                assert_eq!(
                    e.document.project.output.film_preset,
                    FilmPreset::MediumFormat
                );
                assert_eq!((e.document.film_emulation, e.document.halation), previous);
            });
            h.enter(|e, cx| e.restore_history(super::super::history::Direction::Redo, cx));
            wait_for(&mut h, |e| !e.busy);
            h.read(|e| {
                assert_eq!(
                    e.document.project.output.film_preset,
                    FilmPreset::ThirtyFiveMm
                );
                assert!(e.document.film_emulation.enabled && e.document.halation.enabled);
                assert!(
                    FilmPreset::ThirtyFiveMm
                        .matches(e.document.film_emulation, e.document.halation)
                );
            });
            select_preset(&mut h, "Medium format");
            enter_numeric(&mut h, "Amount", "0.071");
            h.try_run_realtime().unwrap();
            select_preset(&mut h, "Medium format");
            h.read(|e| {
                assert_eq!(
                    e.document.film_emulation.amount, 0.065,
                    "reapplying the selected preset restores actual starting values"
                )
            });
            painted(&mut h);
            if scale == 1. {
                save_capture(&h.render().unwrap(), "film-medium-format-offscreen");
            }
        });
    }
}
