use super::*;

fn frame(
    context: &egui::Context,
    value: &mut f64,
    kind: Parameter,
    width: f32,
    enabled: bool,
    events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
) -> (Response, Response, Response) {
    frame_at(
        context, value, kind, width, enabled, events, modifiers, None,
    )
}

#[allow(clippy::too_many_arguments)]
fn frame_at(
    context: &egui::Context,
    value: &mut f64,
    kind: Parameter,
    width: f32,
    enabled: bool,
    mut events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
    time: Option<f64>,
) -> (Response, Response, Response) {
    super::super::presentation::configure_fonts(context);
    let mut responses = None;
    events.insert(0, egui::Event::ModifiersChanged(modifiers));
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 400.),
            )),
            events,
            time,
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

/// Option-click on the value name: the single-action reset gesture.
fn alt_click(position: egui::Pos2) -> Vec<egui::Event> {
    let mut events = vec![egui::Event::PointerMoved(position)];
    for pressed in [true, false] {
        events.push(egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::ALT,
        });
    }
    events
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
fn inward_margin_uses_shared_precision_keyboard_steps_and_actual_default() {
    for width in [315., 270.] {
        let context = egui::Context::default();
        let mut value = 1.23456789;
        let (slider, numeric, label) = frame(
            &context,
            &mut value,
            Parameter::CropMargin,
            width,
            true,
            vec![],
            egui::Modifiers::NONE,
        );
        assert_eq!(value, 1.23456789);
        assert!(slider.rect.right() <= numeric.rect.left());
        assert!(
            slider.rect.is_positive() && slider.rect.left() >= 0. && slider.rect.right() <= width
        );
        frame(
            &context,
            &mut value,
            Parameter::CropMargin,
            width,
            true,
            alt_click(label.rect.center()),
            egui::Modifiers::ALT,
        );
        assert_eq!(value, crate::auto_crop::DEFAULT_MARGIN_PERCENT);

        // Tab reaches the track first; the shared step then moves it by one increment.
        frame(
            &context,
            &mut value,
            Parameter::CropMargin,
            width,
            true,
            vec![key(egui::Key::Tab, egui::Modifiers::NONE)],
            egui::Modifiers::NONE,
        );
        frame(
            &context,
            &mut value,
            Parameter::CropMargin,
            width,
            true,
            vec![key(egui::Key::ArrowRight, egui::Modifiers::NONE)],
            egui::Modifiers::NONE,
        );
        assert!(
            (value - crate::auto_crop::DEFAULT_MARGIN_PERCENT - 0.01).abs() < 1e-12,
            "{value}"
        );
    }
}

#[test]
fn drawing_preserves_manual_precision_and_exposure_outside_normal_slider_range() {
    for width in [270., 315.] {
        for initial in [-16., -7.123456789, 0.123456789, 16.] {
            let context = egui::Context::default();
            let mut value = initial;
            let (slider, numeric, _) = frame(
                &context,
                &mut value,
                Parameter::Exposure,
                width,
                true,
                vec![],
                egui::Modifiers::NONE,
            );
            assert_eq!(value, initial);
            assert!(
                slider.rect.is_positive()
                    && slider.rect.left() >= 0.
                    && slider.rect.right() <= width
            );
            assert!(slider.rect.right() <= numeric.rect.left());
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
fn mask_parameters_preserve_precision_clamp_edits_and_reset_only_when_enabled() {
    for width in [270., 315.] {
        for (kind, text, expected) in [Parameter::Strength, Parameter::Feather]
            .into_iter()
            .flat_map(|kind| {
                [("-0.1", 0.), ("0.123456789", 0.123456789), ("1.1", 1.)]
                    .map(|(text, expected)| (kind, text, expected))
            })
        {
            let context = egui::Context::default();
            let mut value = 0.3456789123;
            let draw = |value: &mut f64, enabled, events, modifiers| {
                frame(&context, value, kind, width, enabled, events, modifiers)
            };
            let response = draw(&mut value, true, vec![], egui::Modifiers::NONE);
            assert_eq!(value, 0.3456789123);
            assert!(response.0.rect.right() <= response.1.rect.left());
            for pressed in [true, false] {
                draw(
                    &mut value,
                    true,
                    pointer(response.1.rect.center(), pressed),
                    egui::Modifiers::NONE,
                );
            }
            draw(
                &mut value,
                true,
                vec![egui::Event::Text(text.into())],
                egui::Modifiers::NONE,
            );
            assert_eq!(value, 0.3456789123);
            draw(
                &mut value,
                true,
                vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
                egui::Modifiers::NONE,
            );
            draw(&mut value, true, vec![], egui::Modifiers::NONE);
            assert_eq!(value, expected);
            value = 0.3456789123;
            let response = draw(&mut value, true, vec![], egui::Modifiers::NONE);
            draw(
                &mut value,
                true,
                alt_click(response.2.rect.center()),
                egui::Modifiers::ALT,
            );
            assert_eq!(value, kind.default(), "reset did not apply");
            draw(
                &mut value,
                true,
                pointer(response.1.rect.center(), true),
                egui::Modifiers::NONE,
            );
            draw(
                &mut value,
                true,
                pointer(response.1.rect.center(), false),
                egui::Modifiers::NONE,
            );
            draw(
                &mut value,
                true,
                vec![egui::Event::Text("0.75".into())],
                egui::Modifiers::NONE,
            );
            draw(&mut value, false, vec![], egui::Modifiers::NONE);
            assert_eq!(
                value,
                kind.default(),
                "disabling must not commit pending mask-parameter text"
            );
        }
    }
}

#[test]
fn parameter_rows_share_one_track_start() {
    let context = egui::Context::default();
    let mut contrast = 1.;
    let mut pivot = 0.18;
    let mut exposure = 0.;
    let mut margin = 1.;
    let mut rects = None;
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(315., 240.),
            )),
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let a = parameter(ui, Parameter::Contrast, &mut contrast).0;
                let b = parameter(ui, Parameter::Pivot, &mut pivot).0;
                let c = parameter(ui, Parameter::Exposure, &mut exposure).0;
                let d = parameter(ui, Parameter::CropMargin, &mut margin).0;
                rects = Some((a, b, c, d));
            });
        },
    );
    output.textures_delta.clear();
    let (contrast, pivot, exposure, margin) = rects.unwrap();
    // Unit labels shorten the track at its right end, but every track starts in the same column.
    assert_eq!(contrast.rect.left(), pivot.rect.left());
    assert_eq!(pivot.rect.left(), exposure.rect.left());
    assert_eq!(exposure.rect.left(), margin.rect.left());
}

#[test]
fn double_clicking_the_name_resets_the_value() {
    let context = egui::Context::default();
    let mut value = 0.5;
    let (_, _, label) = frame(
        &context,
        &mut value,
        Parameter::Exposure,
        315.,
        true,
        vec![],
        egui::Modifiers::NONE,
    );
    let point = label.rect.center();
    // Two clicks inside egui's double-click interval, with time advancing between frames.
    for (time, pressed) in [(0., true), (0., false), (0.1, true), (0.1, false)] {
        frame_at(
            &context,
            &mut value,
            Parameter::Exposure,
            315.,
            true,
            pointer(point, pressed),
            egui::Modifiers::NONE,
            Some(time),
        );
    }
    assert_eq!(value, 0., "double-clicking the name resets to the default");
}

#[test]
fn focus_alone_does_not_round_the_stored_value() {
    let context = egui::Context::default();
    let precise = 0.123456789;
    let mut value = precise;
    let (_, numeric, _) = frame(
        &context,
        &mut value,
        Parameter::Exposure,
        315.,
        true,
        vec![],
        egui::Modifiers::NONE,
    );
    let point = numeric.rect.center();
    for pressed in [true, false] {
        frame(
            &context,
            &mut value,
            Parameter::Exposure,
            315.,
            true,
            pointer(point, pressed),
            egui::Modifiers::NONE,
        );
    }
    for pressed in [true, false] {
        frame(
            &context,
            &mut value,
            Parameter::Exposure,
            315.,
            true,
            pointer(egui::pos2(4., 4.), pressed),
            egui::Modifiers::NONE,
        );
    }
    assert_eq!(value, precise, "an inert focus round-trip keeps the value");
}

#[test]
fn an_interrupted_precision_drag_does_not_pin_the_value() {
    let context = egui::Context::default();
    let mut value = 0.;
    let (slider, _, _) = frame(
        &context,
        &mut value,
        Parameter::Exposure,
        315.,
        true,
        vec![],
        egui::Modifiers::NONE,
    );
    let start = slider.rect.left_center() + egui::vec2(40., 0.);
    frame(
        &context,
        &mut value,
        Parameter::Exposure,
        315.,
        true,
        pointer(start, true),
        egui::Modifiers::SHIFT,
    );
    frame(
        &context,
        &mut value,
        Parameter::Exposure,
        315.,
        true,
        vec![egui::Event::PointerMoved(start + egui::vec2(10., 0.))],
        egui::Modifiers::SHIFT,
    );
    let dragged = value;
    assert_ne!(dragged, 0.);
    // The row disappears mid-gesture and the pointer is released elsewhere. When it returns,
    // the pre-drag snapshot must not win over the accepted value.
    let mut other = 1.;
    frame(
        &context,
        &mut other,
        Parameter::Contrast,
        315.,
        true,
        pointer(egui::pos2(4., 4.), false),
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
    assert_eq!(value, dragged);
}

#[test]
fn shift_drag_on_the_track_moves_ten_times_finer() {
    let drag = |modifiers: egui::Modifiers| {
        let context = egui::Context::default();
        let mut value = 0.;
        let (slider, _, _) = frame(
            &context,
            &mut value,
            Parameter::Exposure,
            315.,
            true,
            vec![],
            modifiers,
        );
        let start = slider.rect.left_center() + egui::vec2(40., 0.);
        frame(
            &context,
            &mut value,
            Parameter::Exposure,
            315.,
            true,
            pointer(start, true),
            modifiers,
        );
        let pressed = value;
        frame(
            &context,
            &mut value,
            Parameter::Exposure,
            315.,
            true,
            vec![egui::Event::PointerMoved(start + egui::vec2(10., 0.))],
            modifiers,
        );
        (pressed, value)
    };
    let (plain_press, plain) = drag(egui::Modifiers::NONE);
    let (fine_press, fine) = drag(egui::Modifiers::SHIFT);
    let plain_travel = plain - plain_press;
    let fine_travel = fine - fine_press;
    assert!(plain_travel.abs() > 0.1, "{plain_travel}");
    assert_eq!(fine_press, 0., "a precision press holds the accepted value");
    // Same direction, materially slower; the exact factor is a per-pixel implementation
    // detail and differs for logarithmic tracks.
    assert!(
        fine_travel.signum() == plain_travel.signum()
            && fine_travel.abs() < plain_travel.abs() / 5.
            && fine_travel.abs() > plain_travel.abs() / 20.,
        "fine {fine_travel} vs ordinary {plain_travel}"
    );
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
        vec![egui::Event::Text("-8".into())],
        egui::Modifiers::NONE,
    );
    frame(
        &context,
        &mut value,
        Parameter::Exposure,
        315.,
        true,
        vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
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
    // Tab order follows the row: the track first, then the value field.
    for _ in 0..2 {
        frame(
            &context,
            &mut value,
            Parameter::Exposure,
            315.,
            true,
            vec![key(egui::Key::Tab, egui::Modifiers::NONE)],
            egui::Modifiers::NONE,
        );
    }
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
            alt_click(position),
            egui::Modifiers::ALT,
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
fn contrast_rows_accept_wider_endpoints_and_reset_to_identity() {
    assert_eq!(Parameter::Contrast.range(), 0.4..=1.6);
    for (initial, edge_key) in [(0.4, egui::Key::ArrowLeft), (1.6, egui::Key::ArrowRight)] {
        let context = egui::Context::default();
        let mut value = initial;
        let (slider, _, label) = frame(
            &context,
            &mut value,
            Parameter::Contrast,
            270.,
            true,
            vec![],
            egui::Modifiers::NONE,
        );
        assert_eq!(value, initial);
        context.memory_mut(|memory| memory.request_focus(slider.id));
        frame(
            &context,
            &mut value,
            Parameter::Contrast,
            270.,
            true,
            vec![key(edge_key, egui::Modifiers::NONE)],
            egui::Modifiers::NONE,
        );
        assert_eq!(value, initial);
        frame(
            &context,
            &mut value,
            Parameter::Contrast,
            270.,
            true,
            alt_click(label.rect.center()),
            egui::Modifiers::ALT,
        );
        assert_eq!(value, 1.);
    }
}

#[test]
fn photographic_blend_rows_fit_and_reset_to_zero_only_when_enabled() {
    for kind in [Parameter::ColourMix, Parameter::Density] {
        assert_eq!(kind.default(), 0.);
        for width in [270., 315.] {
            for enabled in [true, false] {
                let context = egui::Context::default();
                let mut value = 0.75;
                let (_, numeric, reset) = frame(
                    &context,
                    &mut value,
                    kind,
                    width,
                    enabled,
                    vec![],
                    egui::Modifiers::NONE,
                );
                assert!(reset.rect.left() < numeric.rect.left());
                frame(
                    &context,
                    &mut value,
                    kind,
                    width,
                    enabled,
                    alt_click(reset.rect.center()),
                    egui::Modifiers::ALT,
                );
                assert_eq!(value, if enabled { 0. } else { 0.75 });
            }
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
            frame(
                &context,
                &mut value,
                kind,
                270.,
                enabled,
                alt_click(response.2.rect.center()),
                egui::Modifiers::ALT,
            );
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

/// Test wheel size; the radius, not the size, drives relative motion.
const TEST_WHEEL: f32 = 76.;
const TEST_RADIUS: f32 = TEST_WHEEL * 0.5 - 2.;

#[test]
fn wheel_press_alone_does_not_move_the_marker_and_disabled_wheels_are_read_only() {
    for kind in [ColorControl::GammaBalance, ColorControl::WhiteBalance] {
        for enabled in [true, false] {
            let context = egui::Context::default();
            let untouched = Balance {
                warmth: 0.21,
                tint: -0.13,
            };
            let mut balance = untouched;
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
                            response = Some(wheel(ui, kind, &mut balance, TEST_WHEEL));
                        });
                    },
                );
                output.textures_delta.clear();
                response.as_ref().unwrap().rect.center()
            };
            let center = render(vec![]);
            render(pointer(center + egui::vec2(26., 0.), true));
            render(pointer(center + egui::vec2(26., 0.), false));
            // Pressing away from the marker is a grab, not a jump; a disabled wheel keeps the
            // accepted value whatever the pointer does.
            assert_balance(balance, untouched);
        }
    }
}

fn wheel_frame(
    context: &egui::Context,
    balance: &mut Balance,
    kind: ColorControl,
    enabled: bool,
    mut events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
) -> Response {
    events.insert(0, egui::Event::ModifiersChanged(modifiers));
    for event in &mut events {
        if let egui::Event::PointerButton {
            modifiers: event_modifiers,
            ..
        } = event
        {
            *event_modifiers = modifiers;
        }
    }
    let mut response = None;
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(315., 250.),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            ui.add_enabled_ui(enabled, |ui| {
                response = Some(wheel(ui, kind, balance, TEST_WHEEL));
            });
        },
    );
    output.textures_delta.clear();
    response.unwrap()
}

fn assert_balance(actual: Balance, expected: Balance) {
    // Relative motion accumulates frame by frame, so exact bit equality is not the contract;
    // the stored value still keeps far more precision than any control can address.
    assert!(
        (actual.warmth - expected.warmth).abs() < 1e-9,
        "{actual:?} != {expected:?}"
    );
    assert!(
        (actual.tint - expected.tint).abs() < 1e-9,
        "{actual:?} != {expected:?}"
    );
}

#[test]
fn shift_wheel_drag_anchors_to_the_current_value_and_uses_one_tenth_motion() {
    for scale in [1., 2.] {
        for kind in [ColorControl::GammaBalance, ColorControl::WhiteBalance] {
            let context = egui::Context::default();
            context.set_pixels_per_point(scale);
            let initial = Balance {
                warmth: 0.234567891234,
                tint: -0.123456789123,
            };
            let mut balance = initial;
            let wheel = wheel_frame(
                &context,
                &mut balance,
                kind,
                true,
                vec![],
                egui::Modifiers::NONE,
            );
            let origin = wheel.rect.center() + egui::vec2(-30., 10.);
            wheel_frame(
                &context,
                &mut balance,
                kind,
                true,
                pointer(origin, true),
                egui::Modifiers::SHIFT,
            );
            assert_eq!(
                balance, initial,
                "precision press does not reposition the marker"
            );
            let point = origin + egui::vec2(2., -1.);
            wheel_frame(
                &context,
                &mut balance,
                kind,
                true,
                vec![egui::Event::PointerMoved(point)],
                egui::Modifiers::SHIFT,
            );
            assert_eq!(
                balance, initial,
                "pointer jitter below the drag threshold is not an edit"
            );
            let point = point + egui::vec2(26., -13.);
            wheel_frame(
                &context,
                &mut balance,
                kind,
                true,
                vec![egui::Event::PointerMoved(point)],
                egui::Modifiers::SHIFT,
            );
            let delta = (point - origin) / TEST_RADIUS;
            let expected = kind.coordinates(
                kind.position(initial) + DVec2::new(f64::from(delta.x), -f64::from(delta.y)) * 0.1,
            );
            assert_balance(balance, expected);
            let before = balance;
            wheel_frame(
                &context,
                &mut balance,
                kind,
                true,
                pointer(point, false),
                egui::Modifiers::SHIFT,
            );
            wheel_frame(
                &context,
                &mut balance,
                kind,
                true,
                vec![],
                egui::Modifiers::NONE,
            );
            assert_eq!(balance, before, "release/idle does not move the marker");
        }
    }
}

#[test]
fn wheel_modifier_changes_keep_motion_relative_until_release() {
    for kind in [ColorControl::GammaBalance, ColorControl::WhiteBalance] {
        let context = egui::Context::default();
        let mut balance = Balance::default();
        let center = wheel_frame(
            &context,
            &mut balance,
            kind,
            true,
            vec![],
            egui::Modifiers::NONE,
        )
        .rect
        .center();
        wheel_frame(
            &context,
            &mut balance,
            kind,
            true,
            pointer(center, true),
            egui::Modifiers::NONE,
        );
        // Each frame contributes its own travel; the sensitivity is per frame, not cumulative.
        let mut point = center;
        for (step, modifiers) in [
            (egui::vec2(18., 0.), egui::Modifiers::NONE),
            (egui::vec2(-9., 9.), egui::Modifiers::SHIFT),
            (egui::vec2(0., -9.), egui::Modifiers::NONE),
            (egui::vec2(9., 0.), egui::Modifiers::SHIFT),
        ] {
            let before = balance;
            let idle = wheel_frame(&context, &mut balance, kind, true, vec![], modifiers);
            assert_eq!(
                balance, before,
                "stationary modifier changes preserve exact f64 values"
            );
            assert_eq!(idle.rect.width(), TEST_WHEEL);
            let next = point + step;
            wheel_frame(
                &context,
                &mut balance,
                kind,
                true,
                vec![egui::Event::PointerMoved(next)],
                modifiers,
            );
            let delta = step / TEST_RADIUS;
            assert_balance(
                balance,
                kind.coordinates(
                    kind.position(before)
                        + DVec2::new(f64::from(delta.x), -f64::from(delta.y))
                            * if modifiers.shift { 0.1 } else { 1. },
                ),
            );
            point = next;
        }
        let before = balance;
        wheel_frame(
            &context,
            &mut balance,
            kind,
            true,
            vec![],
            egui::Modifiers::NONE,
        );
        wheel_frame(
            &context,
            &mut balance,
            kind,
            true,
            pointer(point, false),
            egui::Modifiers::NONE,
        );
        assert_eq!(balance, before);
        // A fresh press starts a new relative gesture instead of jumping to the pointer.
        let target = center + egui::vec2(-26., 0.);
        wheel_frame(
            &context,
            &mut balance,
            kind,
            true,
            pointer(target, true),
            egui::Modifiers::NONE,
        );
        wheel_frame(
            &context,
            &mut balance,
            kind,
            true,
            pointer(target, false),
            egui::Modifiers::NONE,
        );
        assert_eq!(balance, before, "a click without motion is not an edit");
    }
}

#[test]
fn wheel_drag_accumulates_relative_motion_from_the_press_origin() {
    for kind in [ColorControl::GammaBalance, ColorControl::WhiteBalance] {
        let context = egui::Context::default();
        let mut balance = Balance {
            warmth: 0.2,
            tint: -0.1,
        };
        let center = wheel_frame(
            &context,
            &mut balance,
            kind,
            true,
            vec![],
            egui::Modifiers::NONE,
        )
        .rect
        .center();
        let start = center + egui::vec2(-26., 26.);
        let before = balance;
        wheel_frame(
            &context,
            &mut balance,
            kind,
            true,
            pointer(start, true),
            egui::Modifiers::SHIFT,
        );
        wheel_frame(
            &context,
            &mut balance,
            kind,
            true,
            pointer(start, false),
            egui::Modifiers::SHIFT,
        );
        assert_eq!(balance, before, "a precision click does not reposition");

        wheel_frame(
            &context,
            &mut balance,
            kind,
            true,
            pointer(start, true),
            egui::Modifiers::NONE,
        );
        let mut point = start;
        for step in [egui::vec2(26., -13.), egui::vec2(-13., 13.)] {
            let before = balance;
            let next = point + step;
            wheel_frame(
                &context,
                &mut balance,
                kind,
                true,
                vec![egui::Event::PointerMoved(next)],
                egui::Modifiers::NONE,
            );
            let delta = step / TEST_RADIUS;
            assert_balance(
                balance,
                kind.coordinates(
                    kind.position(before) + DVec2::new(f64::from(delta.x), -f64::from(delta.y)),
                ),
            );
            point = next;
        }
    }
}

#[test]
fn disabled_wheels_never_apply_precision_motion_and_discard_drag_memory() {
    for kind in [ColorControl::GammaBalance, ColorControl::WhiteBalance] {
        let context = egui::Context::default();
        let mut balance = Balance {
            warmth: 0.123456789123,
            tint: -0.234567891234,
        };
        let response = wheel_frame(
            &context,
            &mut balance,
            kind,
            true,
            vec![],
            egui::Modifiers::NONE,
        );
        let center = response.rect.center();
        wheel_frame(
            &context,
            &mut balance,
            kind,
            true,
            pointer(center, true),
            egui::Modifiers::SHIFT,
        );
        wheel_frame(
            &context,
            &mut balance,
            kind,
            true,
            vec![egui::Event::PointerMoved(center + egui::vec2(13., 0.))],
            egui::Modifiers::SHIFT,
        );
        let before = balance;
        let disabled = wheel_frame(
            &context,
            &mut balance,
            kind,
            false,
            vec![egui::Event::PointerMoved(center + egui::vec2(26., 13.))],
            egui::Modifiers::SHIFT,
        );
        assert_eq!(disabled.id, response.id);
        assert!(!disabled.enabled());
        assert_eq!(balance, before);
        assert!(
            context
                .data_mut(|data| data.get_temp::<WheelDrag>(response.id.with("wheel-drag")))
                .is_none()
        );
        wheel_frame(
            &context,
            &mut balance,
            kind,
            false,
            pointer(center + egui::vec2(26., 13.), false),
            egui::Modifiers::SHIFT,
        );
        wheel_frame(
            &context,
            &mut balance,
            kind,
            false,
            pointer(center, true),
            egui::Modifiers::SHIFT,
        );
        wheel_frame(
            &context,
            &mut balance,
            kind,
            false,
            vec![egui::Event::PointerMoved(center + egui::vec2(26., 0.))],
            egui::Modifiers::SHIFT,
        );
        wheel_frame(
            &context,
            &mut balance,
            kind,
            false,
            pointer(center + egui::vec2(26., 0.), false),
            egui::Modifiers::SHIFT,
        );
        assert_eq!(balance, before);
        wheel_frame(
            &context,
            &mut balance,
            kind,
            true,
            vec![],
            egui::Modifiers::NONE,
        );
        wheel_frame(
            &context,
            &mut balance,
            kind,
            true,
            pointer(center, true),
            egui::Modifiers::NONE,
        );
        wheel_frame(
            &context,
            &mut balance,
            kind,
            true,
            pointer(center, false),
            egui::Modifiers::NONE,
        );
        assert_eq!(balance, before, "a disabled gesture leaves the value alone");
    }
}

#[test]
fn precision_wheels_keep_existing_coordinate_bounds_and_use_current_accepted_values() {
    for kind in [ColorControl::GammaBalance, ColorControl::WhiteBalance] {
        let context = egui::Context::default();
        let mut balance = Balance::default();
        let center = wheel_frame(
            &context,
            &mut balance,
            kind,
            true,
            vec![],
            egui::Modifiers::NONE,
        )
        .rect
        .center();
        wheel_frame(
            &context,
            &mut balance,
            kind,
            true,
            pointer(center, true),
            egui::Modifiers::SHIFT,
        );
        let point = center + egui::vec2(5200., -5200.);
        wheel_frame(
            &context,
            &mut balance,
            kind,
            true,
            vec![egui::Event::PointerMoved(point)],
            egui::Modifiers::SHIFT,
        );
        assert!(balance.validate().is_ok());
        assert_eq!(balance.warmth.abs(), 1.);
        assert_eq!(balance.tint.abs(), 1.);
        let accepted = Balance {
            warmth: 0.123456789123,
            tint: -0.234567891234,
        };
        balance = accepted;
        wheel_frame(
            &context,
            &mut balance,
            kind,
            true,
            vec![],
            egui::Modifiers::SHIFT,
        );
        assert_eq!(
            balance, accepted,
            "an idle wheel never overrides the accepted value"
        );
        let anchor = center + egui::vec2(0., 40.);
        wheel_frame(
            &context,
            &mut balance,
            kind,
            true,
            pointer(anchor, true),
            egui::Modifiers::SHIFT,
        );
        wheel_frame(
            &context,
            &mut balance,
            kind,
            true,
            vec![egui::Event::PointerMoved(anchor + egui::vec2(-26., 0.))],
            egui::Modifiers::SHIFT,
        );
        assert_balance(
            balance,
            kind.coordinates(
                kind.position(accepted) + DVec2::new(f64::from(-26. / TEST_RADIUS), 0.) * 0.1,
            ),
        );
    }
}

#[test]
fn composed_gamma_roll_and_photo_wheels_keep_precision_gestures_scoped() {
    let context = egui::Context::default();
    let initial = [
        Balance {
            warmth: 0.1,
            tint: -0.2,
        },
        Balance {
            warmth: -0.2,
            tint: 0.1,
        },
        Balance {
            warmth: 0.3,
            tint: -0.1,
        },
    ];
    context.enable_accesskit();
    let mut balances = initial;
    let draw = |balances: &mut [Balance; 3], mut events: Vec<egui::Event>, modifiers| {
        events.insert(0, egui::Event::ModifiersChanged(modifiers));
        for event in &mut events {
            if let egui::Event::PointerButton {
                modifiers: event_modifiers,
                ..
            } = event
            {
                *event_modifiers = modifiers;
            }
        }
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(315., 1800.),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                balance_body(ui, ColorControl::GammaBalance, &mut balances[0]);
                white_balance(ui, "roll-white-balance", &mut balances[1], false);
                balance_body(ui, ColorControl::WhiteBalance, &mut balances[2]);
            },
        );
        output.textures_delta.clear();
        let mut wheels: Vec<_> = output
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .into_iter()
            .filter_map(|(_, node)| {
                if node.role() == egui::accesskit::Role::Image
                    && matches!(
                        node.label(),
                        Some("Gamma balance wheel" | "White balance wheel")
                    )
                {
                    node.bounds()
                } else {
                    None
                }
            })
            .collect();
        wheels.sort_by(|a, b| a.y0.total_cmp(&b.y0));
        wheels
            .into_iter()
            .map(|bounds| {
                (
                    egui::pos2(
                        ((bounds.x0 + bounds.x1) / 2.) as f32,
                        ((bounds.y0 + bounds.y1) / 2.) as f32,
                    ),
                    ((bounds.x1 - bounds.x0) / 2.) as f32 - 2.,
                )
            })
            .collect::<Vec<_>>()
    };
    draw(&mut balances, vec![], egui::Modifiers::NONE);
    let centers = draw(&mut balances, vec![], egui::Modifiers::NONE);
    assert_eq!(centers.len(), 3);
    for (index, (center, radius)) in centers.into_iter().enumerate() {
        let before = balances;
        let origin = center + egui::vec2(-13., 13.);
        let point = origin + egui::vec2(26., -13.);
        draw(&mut balances, pointer(origin, true), egui::Modifiers::SHIFT);
        draw(
            &mut balances,
            vec![egui::Event::PointerMoved(point)],
            egui::Modifiers::SHIFT,
        );
        draw(&mut balances, pointer(point, false), egui::Modifiers::SHIFT);
        for other in 0..3 {
            if other != index {
                assert_eq!(balances[other], before[other]);
            }
        }
        let kind = if index == 0 {
            ColorControl::GammaBalance
        } else {
            ColorControl::WhiteBalance
        };
        // The radius comes from a pixel-rounded accesskit rect, so allow a sub-pixel error.
        let expected = kind.coordinates(
            kind.position(before[index])
                + DVec2::new(f64::from(26. / radius), f64::from(13. / radius)) * 0.1,
        );
        assert!(
            (balances[index].warmth - expected.warmth).abs() < 1e-3
                && (balances[index].tint - expected.tint).abs() < 1e-3,
            "{:?} != {expected:?}",
            balances[index]
        );
    }
}

#[test]
fn hidden_wheel_drag_memory_cannot_leak_into_a_new_normal_gesture() {
    let context = egui::Context::default();
    let kind = ColorControl::WhiteBalance;
    let mut balance = Balance::default();
    let response = wheel_frame(
        &context,
        &mut balance,
        kind,
        true,
        vec![],
        egui::Modifiers::NONE,
    );
    let center = response.rect.center();
    wheel_frame(
        &context,
        &mut balance,
        kind,
        true,
        pointer(center, true),
        egui::Modifiers::SHIFT,
    );
    wheel_frame(
        &context,
        &mut balance,
        kind,
        true,
        vec![egui::Event::PointerMoved(center + egui::vec2(26., 0.))],
        egui::Modifiers::SHIFT,
    );
    let mut output = context.run_ui(
        egui::RawInput {
            events: pointer(center + egui::vec2(26., 0.), false),
            ..Default::default()
        },
        |_| (),
    );
    output.textures_delta.clear();
    wheel_frame(
        &context,
        &mut balance,
        kind,
        true,
        vec![],
        egui::Modifiers::NONE,
    );
    let target = center + egui::vec2(-26., 0.);
    wheel_frame(
        &context,
        &mut balance,
        kind,
        true,
        pointer(target, true),
        egui::Modifiers::NONE,
    );
    wheel_frame(
        &context,
        &mut balance,
        kind,
        true,
        pointer(target, false),
        egui::Modifiers::NONE,
    );
    assert_balance(
        balance,
        kind.coordinates(
            kind.position(Balance::default()) + DVec2::new(f64::from(26. / TEST_RADIUS), 0.) * 0.1,
        ),
    );
    assert!(
        context
            .data_mut(|data| data.get_temp::<WheelDrag>(response.id.with("wheel-drag")))
            .is_none()
    );
}

#[test]
fn precision_motion_is_not_applied_twice_when_egui_repeats_a_layout_pass() {
    for kind in [ColorControl::GammaBalance, ColorControl::WhiteBalance] {
        let context = egui::Context::default();
        let initial = Balance {
            warmth: 0.123456789123,
            tint: -0.234567891234,
        };
        let mut balance = initial;
        let center = wheel_frame(
            &context,
            &mut balance,
            kind,
            true,
            vec![],
            egui::Modifiers::NONE,
        )
        .rect
        .center();
        wheel_frame(
            &context,
            &mut balance,
            kind,
            true,
            pointer(center, true),
            egui::Modifiers::SHIFT,
        );
        let mut passes = 0;
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(315., 250.),
                )),
                events: vec![
                    egui::Event::ModifiersChanged(egui::Modifiers::SHIFT),
                    egui::Event::PointerMoved(center + egui::vec2(26., 0.)),
                ],
                ..Default::default()
            },
            |ui| {
                ui.add_enabled_ui(true, |ui| {
                    wheel(ui, kind, &mut balance, TEST_WHEEL);
                });
                passes += 1;
                if passes == 1 {
                    ui.ctx()
                        .request_discard("test wheel precision across layout passes");
                }
            },
        );
        output.textures_delta.clear();
        assert!(passes > 1);
        assert_balance(
            balance,
            kind.coordinates(
                kind.position(initial) + DVec2::new(f64::from(26. / TEST_RADIUS), 0.) * 0.1,
            ),
        );
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

struct SectionFrame {
    numeric: Option<Response>,
    other: Option<Response>,
    output: egui::FullOutput,
}

fn section_frame(
    context: &egui::Context,
    value: &mut f64,
    events: Vec<egui::Event>,
) -> SectionFrame {
    context.enable_accesskit();
    let mut numeric = None;
    let mut other = None;
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(315., 800.),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let changed = *value != 0.;
                let (_, action) = section(
                    ui,
                    "grade",
                    "Grade",
                    SectionHeader {
                        summary: &format!("{:+.2} EV", value),
                        changed,
                        reset: Some(changed),
                        tool: None,
                        default_open: Some(true),
                    },
                    |ui| {
                        numeric = Some(parameter(ui, Parameter::Exposure, value).1);
                    },
                );
                if action.reset {
                    *value = 0.;
                }
                section(
                    ui,
                    "other",
                    "Other",
                    SectionHeader {
                        summary: "Independent section",
                        changed: false,
                        reset: None,
                        tool: None,
                        default_open: Some(true),
                    },
                    |ui| {
                        other = Some(ui.button("Other body"));
                    },
                );
            });
        },
    );
    output.textures_delta.clear();
    SectionFrame {
        numeric,
        other,
        output,
    }
}

fn named_rect(output: &egui::FullOutput, name: &str) -> egui::Rect {
    let node = &output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some(name))
        .unwrap()
        .1;
    let rect = node.bounds().unwrap();
    egui::Rect::from_min_max(
        egui::pos2(rect.x0 as f32, rect.y0 as f32),
        egui::pos2(rect.x1 as f32, rect.y1 as f32),
    )
}

#[test]
fn collapsed_sections_keep_adjustments_summaries_and_independent_resets() {
    let context = egui::Context::default();
    let mut value = 1.23456789;
    let first = section_frame(&context, &mut value, vec![]);
    let numeric_id = first.numeric.as_ref().unwrap().id;
    let other_id = first.other.as_ref().unwrap().id;
    let header = named_rect(&first.output, "Grade").center();
    for pressed in [true, false] {
        section_frame(&context, &mut value, pointer(header, pressed));
    }
    let collapsed = section_frame(&context, &mut value, vec![]);
    assert!(collapsed.numeric.is_none());
    assert_eq!(collapsed.other.as_ref().unwrap().id, other_id);
    assert_eq!(value, 1.23456789);
    assert!(
        collapsed
            .output
            .shapes
            .iter()
            .any(|shape| match &shape.shape {
                egui::Shape::Text(text) => text.galley.job.text.contains("+1.23 EV"),
                _ => false,
            })
    );
    let modified = |output: &egui::FullOutput| {
        output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Grade"))
            .unwrap()
            .1
            .description()
            == Some("Modified")
    };
    assert!(modified(&collapsed.output));
    value = 2.;
    assert!(
        section_frame(&context, &mut value, vec![])
            .numeric
            .is_none()
    );
    let reset = named_rect(&collapsed.output, "Reset Grade").center();
    for pressed in [true, false] {
        section_frame(&context, &mut value, pointer(reset, pressed));
    }
    let reset = section_frame(&context, &mut value, vec![]);
    assert_eq!(value, 0.);
    assert!(reset.numeric.is_none());
    assert!(!modified(&reset.output));
    for pressed in [true, false] {
        section_frame(&context, &mut value, pointer(header, pressed));
    }
    let reopened = section_frame(&context, &mut value, vec![]);
    assert_eq!(reopened.numeric.as_ref().unwrap().id, numeric_id);
    assert!(reopened.other.is_some());
}

#[test]
fn collapsing_a_pending_numeric_edit_cancels_without_committing_on_reopen() {
    let context = egui::Context::default();
    let mut value = 0.123456789;
    let first = section_frame(&context, &mut value, vec![]);
    let numeric = first.numeric.unwrap().rect.center();
    let header = named_rect(&first.output, "Grade").center();
    for pressed in [true, false] {
        section_frame(&context, &mut value, pointer(numeric, pressed));
    }
    section_frame(&context, &mut value, vec![egui::Event::Text("5".into())]);
    assert_eq!(value, 0.123456789);
    for pressed in [true, false] {
        section_frame(&context, &mut value, pointer(header, pressed));
    }
    for _ in 0..3 {
        assert!(
            section_frame(&context, &mut value, vec![])
                .numeric
                .is_none()
        );
        assert_eq!(value, 0.123456789);
    }
    for pressed in [true, false] {
        section_frame(&context, &mut value, pointer(header, pressed));
    }
    let reopened = section_frame(&context, &mut value, vec![]);
    assert!(reopened.numeric.is_some());
    assert_eq!(value, 0.123456789);
    for pressed in [true, false] {
        section_frame(
            &context,
            &mut value,
            pointer(reopened.numeric.as_ref().unwrap().rect.center(), pressed),
        );
    }
    section_frame(
        &context,
        &mut value,
        vec![
            egui::Event::Text("-2".into()),
            key(egui::Key::Enter, egui::Modifiers::NONE),
        ],
    );
    section_frame(&context, &mut value, vec![]);
    assert_eq!(value, -2.);
}

#[test]
fn disclosure_supports_keyboard_activation_and_reports_expansion() {
    let context = egui::Context::default();
    let mut value = 0.;
    let first = section_frame(&context, &mut value, vec![]);
    let header = named_rect(&first.output, "Grade").center();
    for pressed in [true, false] {
        section_frame(&context, &mut value, pointer(header, pressed));
    }
    assert!(
        section_frame(&context, &mut value, vec![])
            .numeric
            .is_none()
    );
    section_frame(
        &context,
        &mut value,
        vec![key(egui::Key::Tab, egui::Modifiers::NONE)],
    );
    section_frame(
        &context,
        &mut value,
        vec![key(egui::Key::Space, egui::Modifiers::NONE)],
    );
    let expanded = section_frame(&context, &mut value, vec![]);
    assert!(expanded.numeric.is_some());
    let node = &expanded
        .output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("Grade"))
        .unwrap()
        .1;
    assert_eq!(node.is_expanded(), Some(true));
}

#[test]
fn outline_icon_buttons_have_named_hit_targets_and_keyboard_actions() {
    let context = egui::Context::default();
    context.enable_accesskit();
    let mut selected = false;
    let mut render = |events| {
        let mut response = None;
        let mut output = context.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                response = Some(icons::toggle(
                    ui,
                    Icon::Guides,
                    &mut selected,
                    "Guides",
                    "Show composition guides",
                ));
            },
        );
        output.textures_delta.clear();
        (response.unwrap(), output)
    };
    let (response, output) = render(vec![]);
    assert_eq!(response.rect.size(), egui::Vec2::splat(28.));
    assert_eq!(named_rect(&output, "Guides"), response.rect);
    assert!(
        output
            .shapes
            .iter()
            .any(|shape| matches!(&shape.shape, egui::Shape::Path(_)))
    );
    context.memory_mut(|memory| memory.request_focus(response.id));
    render(vec![key(egui::Key::Space, egui::Modifiers::NONE)]);
    drop(render);
    assert!(selected);
}

#[test]
fn disabled_outline_icons_are_dimmed_and_cannot_activate() {
    let context = egui::Context::default();
    let render = |enabled, events| {
        let mut response = None;
        let mut output = context.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                response = Some(icons::button(
                    ui,
                    Icon::Reset,
                    enabled,
                    "Reset",
                    "Reset value",
                ));
            },
        );
        output.textures_delta.clear();
        let stroke = output
            .shapes
            .into_iter()
            .find_map(|shape| {
                if let egui::Shape::Path(path) = shape.shape {
                    Some(path.stroke.color)
                } else {
                    None
                }
            })
            .unwrap();
        (response.unwrap(), stroke)
    };
    let (enabled, normal) = render(true, vec![]);
    let (disabled, dimmed) = render(false, vec![]);
    assert_eq!(enabled.id, disabled.id);
    assert_eq!(enabled.rect, disabled.rect);
    assert_ne!(normal, dimmed);
    assert!(!disabled.enabled());
    for pressed in [true, false] {
        assert!(
            !render(false, pointer(disabled.rect.center(), pressed))
                .0
                .clicked()
        );
    }
}

#[test]
fn value_activity_is_frame_local_and_ends_when_controls_are_hidden_or_disabled() {
    let context = egui::Context::default();
    let mut value = 0.;
    let mut render = |visible: bool, enabled: bool, events| {
        let mut slider = None;
        let mut active = false;
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(315., 200.),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                if visible {
                    slider = Some(
                        ui.add_enabled_ui(enabled, |ui| {
                            parameter(ui, Parameter::Exposure, &mut value).0
                        })
                        .inner,
                    );
                }
                active = values_are_being_adjusted(ui.ctx());
            },
        );
        output.textures_delta.clear();
        (slider, active)
    };
    let (slider, active) = render(true, true, vec![]);
    assert!(!active);
    let point = slider.unwrap().rect.center();
    assert!(render(true, true, pointer(point, true)).1);
    assert!(render(true, true, vec![]).1);
    assert!(!render(false, true, vec![]).1);
    assert!(!render(true, false, vec![]).1);
    assert!(!render(true, true, pointer(point, false)).1);
}
