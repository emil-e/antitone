use super::*;

fn frame(
    context: &egui::Context,
    value: &mut f64,
    kind: Parameter,
    width: f32,
    enabled: bool,
    mut events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
) -> (Response, Response, Response) {
    let mut responses = None;
    events.insert(0, egui::Event::ModifiersChanged(modifiers));
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 400.),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.add_enabled_ui(enabled, |ui| {
                    responses = Some(parameter(ui, kind, value));
                });
            });
        },
    );
    output.textures_delta.clear();
    responses.unwrap()
}
fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}
fn pointer(position: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(position),
        egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}
#[test]
fn drawing_preserves_manual_precision_and_exposure_outside_normal_slider_range() {
    for width in [230., 315.] {
        for initial in [-16., -7.123456789, 0.123456789, 16.] {
            let context = egui::Context::default();
            let mut value = initial;
            let (slider, numeric, reset) = frame(
                &context,
                &mut value,
                Parameter::Exposure,
                width,
                true,
                vec![],
                egui::Modifiers::NONE,
            );
            assert_eq!(value, initial);
            assert!((slider.rect.width() - (width - 16.)).abs() < 1.);
            assert!(numeric.rect.right() < reset.rect.left());
            assert!(numeric.rect.bottom() <= slider.rect.top());
            let second = frame(
                &context,
                &mut value,
                Parameter::Exposure,
                width,
                true,
                vec![],
                egui::Modifiers::NONE,
            );
            assert_eq!(second.0.id, slider.id);
            assert_eq!(second.1.id, numeric.id);
            assert_eq!(value, initial);
        }
    }
}
#[test]
fn slider_keyboard_steps_are_precise_and_shift_is_finer() {
    for (modifiers, expected) in [
        (egui::Modifiers::NONE, 0.05),
        (egui::Modifiers::SHIFT, 0.005),
    ] {
        let context = egui::Context::default();
        let mut value = 0.;
        let response = frame(
            &context,
            &mut value,
            Parameter::Exposure,
            315.,
            true,
            vec![],
            modifiers,
        );
        context.memory_mut(|m| m.request_focus(response.0.id));
        frame(
            &context,
            &mut value,
            Parameter::Exposure,
            315.,
            true,
            vec![key(egui::Key::ArrowRight, modifiers)],
            modifiers,
        );
        assert!((value - expected).abs() < 1e-12, "{value}");
    }
}
#[test]
fn numeric_entry_commits_full_range_and_escape_cancels_without_hidden_rounding() {
    let context = egui::Context::default();
    let mut value = 0.123456789;
    let response = frame(
        &context,
        &mut value,
        Parameter::Exposure,
        315.,
        true,
        vec![],
        egui::Modifiers::NONE,
    );
    for pressed in [true, false] {
        frame(
            &context,
            &mut value,
            Parameter::Exposure,
            315.,
            true,
            pointer(response.1.rect.center(), pressed),
            egui::Modifiers::NONE,
        );
    }
    frame(
        &context,
        &mut value,
        Parameter::Exposure,
        315.,
        true,
        vec![egui::Event::Text("12.345678".into())],
        egui::Modifiers::NONE,
    );
    assert_eq!(value, 0.123456789);
    frame(
        &context,
        &mut value,
        Parameter::Exposure,
        315.,
        true,
        vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
        egui::Modifiers::NONE,
    );
    frame(
        &context,
        &mut value,
        Parameter::Exposure,
        315.,
        true,
        vec![],
        egui::Modifiers::NONE,
    );
    assert_eq!(value, 12.345678);
    for pressed in [true, false] {
        frame(
            &context,
            &mut value,
            Parameter::Exposure,
            315.,
            true,
            pointer(response.1.rect.center(), pressed),
            egui::Modifiers::NONE,
        );
    }
    frame(
        &context,
        &mut value,
        Parameter::Exposure,
        315.,
        true,
        vec![
            egui::Event::Text("-8".into()),
            key(egui::Key::Escape, egui::Modifiers::NONE),
        ],
        egui::Modifiers::NONE,
    );
    frame(
        &context,
        &mut value,
        Parameter::Exposure,
        315.,
        true,
        vec![],
        egui::Modifiers::NONE,
    );
    assert_eq!(value, 12.345678);
}
#[test]
fn tab_focus_allows_numeric_entry_without_a_mouse() {
    let context = egui::Context::default();
    let mut value = 0.;
    frame(
        &context,
        &mut value,
        Parameter::Exposure,
        315.,
        true,
        vec![],
        egui::Modifiers::NONE,
    );
    frame(
        &context,
        &mut value,
        Parameter::Exposure,
        315.,
        true,
        vec![key(egui::Key::Tab, egui::Modifiers::NONE)],
        egui::Modifiers::NONE,
    );
    frame(
        &context,
        &mut value,
        Parameter::Exposure,
        315.,
        true,
        vec![egui::Event::Text("-11.25".into())],
        egui::Modifiers::NONE,
    );
    frame(
        &context,
        &mut value,
        Parameter::Exposure,
        315.,
        true,
        vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
        egui::Modifiers::NONE,
    );
    frame(
        &context,
        &mut value,
        Parameter::Exposure,
        315.,
        true,
        vec![],
        egui::Modifiers::NONE,
    );
    assert_eq!(value, -11.25);
}
#[test]
fn disabling_a_pending_numeric_edit_does_not_commit_it() {
    let context = egui::Context::default();
    let mut value = 0.;
    let response = frame(
        &context,
        &mut value,
        Parameter::Exposure,
        315.,
        true,
        vec![],
        egui::Modifiers::NONE,
    );
    for pressed in [true, false] {
        frame(
            &context,
            &mut value,
            Parameter::Exposure,
            315.,
            true,
            pointer(response.1.rect.center(), pressed),
            egui::Modifiers::NONE,
        );
    }
    frame(
        &context,
        &mut value,
        Parameter::Exposure,
        315.,
        true,
        vec![egui::Event::Text("5".into())],
        egui::Modifiers::NONE,
    );
    frame(
        &context,
        &mut value,
        Parameter::Exposure,
        315.,
        false,
        vec![],
        egui::Modifiers::NONE,
    );
    frame(
        &context,
        &mut value,
        Parameter::Exposure,
        315.,
        false,
        vec![],
        egui::Modifiers::NONE,
    );
    assert_eq!(value, 0.);
}
#[test]
fn parameter_reset_uses_actual_default_and_disabled_controls_do_not_edit() {
    for enabled in [true, false] {
        let context = egui::Context::default();
        let mut value = 1.2;
        let response = frame(
            &context,
            &mut value,
            Parameter::Gamma,
            315.,
            enabled,
            vec![],
            egui::Modifiers::NONE,
        );
        let position = response.2.rect.center();
        frame(
            &context,
            &mut value,
            Parameter::Gamma,
            315.,
            enabled,
            pointer(position, true),
            egui::Modifiers::NONE,
        );
        frame(
            &context,
            &mut value,
            Parameter::Gamma,
            315.,
            enabled,
            pointer(position, false),
            egui::Modifiers::NONE,
        );
        assert_eq!(value, if enabled { DEFAULT_GAMMA } else { 1.2 });
        if !enabled {
            let position = response.0.rect.center();
            frame(
                &context,
                &mut value,
                Parameter::Gamma,
                315.,
                false,
                pointer(position, true),
                egui::Modifiers::NONE,
            );
            frame(
                &context,
                &mut value,
                Parameter::Gamma,
                315.,
                false,
                pointer(position, false),
                egui::Modifiers::NONE,
            );
            assert_eq!(value, 1.2);
        }
    }
}
#[test]
fn response_inputs_preserve_precision_use_true_defaults_and_fine_shoulder_steps() {
    for (kind, initial, step) in [
        (Parameter::Toe, 3.123456789, 0.01),
        (Parameter::Shoulder, 0.0123456789, 0.0001),
    ] {
        for enabled in [true, false] {
            let context = egui::Context::default();
            let mut value = initial;
            let response = frame(
                &context,
                &mut value,
                kind,
                270.,
                enabled,
                vec![],
                egui::Modifiers::NONE,
            );
            assert_eq!(value, initial);
            for pressed in [true, false] {
                frame(
                    &context,
                    &mut value,
                    kind,
                    270.,
                    enabled,
                    pointer(response.2.rect.center(), pressed),
                    egui::Modifiers::NONE,
                );
            }
            assert_eq!(value, if enabled { kind.default() } else { initial });
        }
        for modifiers in [egui::Modifiers::NONE, egui::Modifiers::SHIFT] {
            let context = egui::Context::default();
            let mut value = kind.default();
            let response = frame(&context, &mut value, kind, 315., true, vec![], modifiers);
            context.memory_mut(|m| m.request_focus(response.0.id));
            frame(
                &context,
                &mut value,
                kind,
                315.,
                true,
                vec![key(egui::Key::ArrowRight, modifiers)],
                modifiers,
            );
            let expected = kind.default() + step * if modifiers.shift { 0.1 } else { 1. };
            assert!((value - expected).abs() < 1e-12, "{value} != {expected}");
        }
    }
}

#[test]
fn wheel_adapters_roundtrip_full_coordinate_square_and_do_not_change_processing() {
    for kind in [ColorControl::GammaBalance, ColorControl::WhiteBalance] {
        for warmth in [-1., -0.3, 0., 0.7, 1.] {
            for tint in [-1., -0.2, 0., 0.6, 1.] {
                let balance = Balance { warmth, tint };
                let position = kind.position(balance);
                assert!(position.length() <= 1. + 1e-12);
                let recovered = kind.coordinates(position);
                assert!((balance.warmth - recovered.warmth).abs() < 1e-12);
                assert!((balance.tint - recovered.tint).abs() < 1e-12);
                assert!(
                    (balance.offsets().unwrap() - recovered.offsets().unwrap()).length() < 1e-12
                );
            }
        }
        assert_eq!(kind.position(Balance::default()), DVec2::ZERO);
        assert!(kind.coordinates(DVec2::splat(10.)).validate().is_ok());
    }
    let balance = Balance {
        warmth: 0.3,
        tint: 0.2,
    };
    assert!(
        (ColorControl::GammaBalance.position(balance)
            - ColorControl::WhiteBalance.position(balance))
        .length()
            > 0.01
    );
}
#[test]
fn wheel_pointer_updates_the_same_coordinates_and_disabled_wheels_are_read_only() {
    for kind in [ColorControl::GammaBalance, ColorControl::WhiteBalance] {
        for enabled in [true, false] {
            let context = egui::Context::default();
            let mut balance = Balance::default();
            let mut response = None;
            let mut render = |events| {
                let mut output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(250., 250.),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        ui.add_enabled_ui(enabled, |ui| {
                            response = Some(wheel(ui, kind, &mut balance));
                        });
                    },
                );
                output.textures_delta.clear();
                response.as_ref().unwrap().rect.center()
            };
            let center = render(vec![]);
            let blue = center + egui::vec2(26., 0.);
            render(pointer(blue, true));
            render(pointer(blue, false));
            let expected = if enabled {
                kind.coordinates(DVec2::X * 0.5)
            } else {
                Balance::default()
            };
            assert!((balance.warmth - expected.warmth).abs() < 1e-7);
            assert!((balance.tint - expected.tint).abs() < 1e-7);
            assert!((kind.position(balance) - kind.position(expected)).length() < 1e-7);
        }
    }
}
#[test]
fn resolve_orientation_keeps_blue_right_and_yellow_left_for_artwork_and_input() {
    assert!((hue_direction(2.) - DVec2::X).length() < 1e-12);
    assert!((hue_direction(5.) + DVec2::X).length() < 1e-12);
    for (rgb, sector) in [(DVec3::X, 0.), (DVec3::Y, 4.), (DVec3::Z, 2.)] {
        let expected = (resolve_orientation() * opponent_coordinates(rgb)).normalize();
        assert!((hue_direction(sector) - expected).length() < 1e-12);
    }
    let blue = ColorControl::GammaBalance
        .coordinates(DVec2::X * 0.1)
        .offsets()
        .unwrap();
    assert!(blue.z > blue.x && blue.z > blue.y);
    assert!((blue.x - blue.y).abs() < 1e-12);
    let yellow = ColorControl::GammaBalance
        .coordinates(-DVec2::X * 0.1)
        .offsets()
        .unwrap();
    assert!(yellow.x > yellow.z && yellow.y > yellow.z);
    assert!((yellow.x - yellow.y).abs() < 1e-12);
}
#[test]
fn wb_wheel_directions_match_local_neutral_response_not_rgb_channel_gains() {
    let epsilon = 1e-7;
    for direction in [DVec2::X, DVec2::Y, DVec2::new(0.5, -0.3)] {
        let coordinates = ColorControl::WhiteBalance.coordinates(direction * epsilon);
        let response = crate::color::wb_matrix(coordinates).unwrap() * DVec3::ONE;
        let actual = resolve_orientation() * opponent_coordinates(response - DVec3::ONE);
        let expected = direction * epsilon * ColorControl::WhiteBalance.scale();
        assert!((actual - expected).length() < 1e-12);
    }
    let red = (resolve_orientation() * opponent_coordinates(DVec3::X)).normalize();
    let gamma = ColorControl::GammaBalance.coordinates(red * 0.1);
    let offsets = gamma.offsets().unwrap();
    assert!(offsets.x > offsets.y && offsets.x > offsets.z);
    assert!((offsets.y - offsets.z).abs() < 1e-12);
}
