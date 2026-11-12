use super::*;
use crate::editor::ui::test_support::pointer;

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
                for (index, value) in balances.iter_mut().enumerate() {
                    match index {
                        0 => {
                            BalanceControl::new("gamma-balance", ColorControl::GammaBalance)
                                .show(ui, value);
                        }
                        1 => {
                            crate::editor::white_balance::controls(
                                ui,
                                "roll-white-balance",
                                value,
                                true,
                                false,
                            );
                        }
                        _ => {
                            BalanceControl::new("photo-balance", ColorControl::WhiteBalance)
                                .show(ui, value);
                        }
                    }
                }
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
