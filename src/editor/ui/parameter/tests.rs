use super::*;
use crate::editor::ui::test_support::{alt_click, key_event as key, pointer, wide_row};
use crate::processing::DEFAULT_GAMMA;

fn identity_row() -> ParameterRow<'static> {
    ParameterRow::new("identity", "Contrast", 0.4..=1.6).default_value(1.)
}

fn percent_row() -> ParameterRow<'static> {
    ParameterRow::new(
        "percent",
        "Margin",
        0.0..=crate::auto_crop::MAX_MARGIN_PERCENT,
    )
    .default_value(crate::auto_crop::DEFAULT_MARGIN_PERCENT)
    .unit("%")
}

fn nonzero_default_row() -> ParameterRow<'static> {
    ParameterRow::new(
        "nonzero",
        "Gamma",
        crate::processing::MIN_GAMMA..=crate::processing::MAX_GAMMA,
    )
    .default_value(DEFAULT_GAMMA)
}

fn frame(
    context: &egui::Context,
    value: &mut f64,
    row: &ParameterRow<'_>,
    width: f32,
    enabled: bool,
    events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
) -> (Response, Response, Response) {
    frame_at(context, value, row, width, enabled, events, modifiers, None)
}

#[allow(clippy::too_many_arguments)]
fn frame_at(
    context: &egui::Context,
    value: &mut f64,
    row: &ParameterRow<'_>,
    width: f32,
    enabled: bool,
    mut events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
    time: Option<f64>,
) -> (Response, Response, Response) {
    crate::editor::ui::configure_fonts(context);
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
                    let response = row.clone().show(ui, value);
                    responses = Some((response.slider, response.numeric, response.label));
                });
            });
        },
    );
    output.textures_delta.clear();
    responses.unwrap()
}

fn configured_rows_frame(
    context: &egui::Context,
    values: &mut [f64; 2],
    label: &str,
    enabled: bool,
    mut events: Vec<egui::Event>,
) -> [ParameterResponse; 2] {
    events.insert(0, egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    let mut responses = None;
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
                let first = ParameterRow::new("first", label, -100.0..=100.0)
                    .slider_range(-1.0..=1.0)
                    .default_value(-12.5)
                    .step(0.25)
                    .decimals(3)
                    .unit("ms")
                    .show(ui, &mut values[0]);
                let second = ParameterRow::new("second", label, 10.0..=20.0)
                    .step(0.5)
                    .decimals(1)
                    .show(ui, &mut values[1]);
                responses = Some([first, second]);
            });
        },
    );
    output.textures_delta.clear();
    responses.unwrap()
}

#[test]
fn configured_rows_keep_same_named_edits_independent_across_label_changes() {
    let context = egui::Context::default();
    let initial = [12.123456789, 18.];
    let mut values = initial;
    let responses = configured_rows_frame(&context, &mut values, "Repeated", true, vec![]);
    assert_eq!(values, initial, "display preserves full stored precision");
    assert_ne!(responses[0].numeric.id, responses[1].numeric.id);
    assert_ne!(responses[0].slider.id, responses[1].slider.id);

    for pressed in [true, false] {
        configured_rows_frame(
            &context,
            &mut values,
            "Repeated",
            true,
            pointer(responses[0].numeric.rect.center(), pressed),
        );
    }
    let renamed = configured_rows_frame(
        &context,
        &mut values,
        "Renamed",
        true,
        vec![egui::Event::Text("63.123456789".into())],
    );
    assert_eq!(renamed[0].numeric.id, responses[0].numeric.id);
    assert_eq!(values, initial, "unfinished text stays private");
    configured_rows_frame(
        &context,
        &mut values,
        "Renamed",
        true,
        vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
    );
    configured_rows_frame(&context, &mut values, "Renamed", true, vec![]);
    assert_eq!(values, [63.123456789, initial[1]]);
}

#[test]
fn configured_rows_own_their_steps_bounds_and_optional_reset_values() {
    let context = egui::Context::default();
    let mut values = [0.75, 18.];
    let responses = configured_rows_frame(&context, &mut values, "Repeated", true, vec![]);
    responses[1].slider.request_focus();
    configured_rows_frame(
        &context,
        &mut values,
        "Repeated",
        true,
        vec![key(egui::Key::ArrowRight, egui::Modifiers::NONE)],
    );
    assert_eq!(values, [0.75, 18.5]);

    // The second row offers no reset; the first has an explicit nonzero default.
    configured_rows_frame(
        &context,
        &mut values,
        "Repeated",
        true,
        alt_click(responses[1].label.rect.center()),
    );
    assert_eq!(values, [0.75, 18.5]);
    let reset = configured_rows_frame(
        &context,
        &mut values,
        "Repeated",
        true,
        alt_click(responses[0].label.rect.center()),
    );
    assert!(reset[0].edit.changed);
    assert_eq!(values, [-12.5, 18.5]);

    values[0] = 0.75;
    configured_rows_frame(
        &context,
        &mut values,
        "Repeated",
        false,
        alt_click(responses[0].label.rect.center()),
    );
    assert_eq!(values, [0.75, 18.5], "inherited disable blocks reset");
    let enabled = configured_rows_frame(&context, &mut values, "Repeated", true, vec![]);
    for pressed in [true, false] {
        configured_rows_frame(
            &context,
            &mut values,
            "Repeated",
            true,
            pointer(enabled[0].numeric.rect.center(), pressed),
        );
    }
    let focused = configured_rows_frame(&context, &mut values, "Repeated", true, vec![]);
    assert!(
        focused[0].numeric.has_focus(),
        "numeric input is admitted after re-enabling"
    );
    configured_rows_frame(
        &context,
        &mut values,
        "Repeated",
        true,
        vec![egui::Event::Text("150".into())],
    );
    configured_rows_frame(
        &context,
        &mut values,
        "Repeated",
        true,
        vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
    );
    configured_rows_frame(&context, &mut values, "Repeated", true, vec![]);
    assert_eq!(
        values,
        [100., 18.5],
        "numeric edits use full bounds, not slider travel"
    );
}

#[test]
fn percentage_rows_use_supplied_defaults_and_precision_keyboard_steps() {
    for width in [315., 270.] {
        let context = egui::Context::default();
        let mut value = 1.23456789;
        let (slider, numeric, label) = frame(
            &context,
            &mut value,
            &percent_row(),
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
            &percent_row(),
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
            &percent_row(),
            width,
            true,
            vec![key(egui::Key::Tab, egui::Modifiers::NONE)],
            egui::Modifiers::NONE,
        );
        frame(
            &context,
            &mut value,
            &percent_row(),
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
fn drawing_preserves_precision_and_values_outside_slider_range() {
    for width in [270., 315.] {
        for initial in [-16., -7.123456789, 0.123456789, 16.] {
            let context = egui::Context::default();
            let mut value = initial;
            let (slider, numeric, _) = frame(
                &context,
                &mut value,
                &wide_row(),
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
                &wide_row(),
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
fn narrow_tracks_preserve_values_beyond_slider_travel_and_accept_full_range_text() {
    for width in [270., 315.] {
        let context = egui::Context::default();
        let row = ParameterRow::new("size", "Size", 0.01..=0.5)
            .slider_range(0.01..=0.21)
            .default_value(0.109)
            .step(0.001)
            .decimals(3);
        let mut value = 0.4;
        let draw = |value: &mut f64, events, modifiers| {
            frame(&context, value, &row, width, true, events, modifiers)
        };
        let (slider, numeric, label) = draw(&mut value, vec![], egui::Modifiers::NONE);
        assert_eq!(value, 0.4, "painting must not clamp an accepted wide halo");
        draw(
            &mut value,
            pointer(slider.rect.center(), true),
            egui::Modifiers::SHIFT,
        );
        assert_eq!(
            value, 0.4,
            "precision press must not jump to the soft bound"
        );
        draw(
            &mut value,
            pointer(slider.rect.center(), false),
            egui::Modifiers::SHIFT,
        );
        draw(
            &mut value,
            alt_click(label.rect.center()),
            egui::Modifiers::ALT,
        );
        assert_eq!(value, 0.109);
        for pressed in [true, false] {
            draw(
                &mut value,
                pointer(numeric.rect.center(), pressed),
                egui::Modifiers::NONE,
            );
        }
        draw(
            &mut value,
            vec![egui::Event::Text("0.4".into())],
            egui::Modifiers::NONE,
        );
        draw(
            &mut value,
            vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
            egui::Modifiers::NONE,
        );
        draw(&mut value, vec![], egui::Modifiers::NONE);
        assert_eq!(value, 0.4, "typing retains access to the full model range");
    }
}

#[test]
fn bounded_rows_preserve_precision_clamp_edits_and_reset_only_when_enabled() {
    for width in [270., 315.] {
        for (default, text, expected) in [1., 0.8].into_iter().flat_map(|default| {
            [("-0.1", 0.), ("0.123456789", 0.123456789), ("1.1", 1.)]
                .map(|(text, expected)| (default, text, expected))
        }) {
            let context = egui::Context::default();
            let mut value = 0.3456789123;
            let draw = |value: &mut f64, enabled, events, modifiers| {
                frame(
                    &context,
                    value,
                    &ParameterRow::new("bounded", "Value", 0.0..=1.0).default_value(default),
                    width,
                    enabled,
                    events,
                    modifiers,
                )
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
            assert_eq!(value, default, "reset did not apply");
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
                value, default,
                "disabling must not commit pending numeric text"
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
                let a = identity_row().show(ui, &mut contrast).slider;
                let b = ParameterRow::new("pivot", "Pivot", 0.01..=1.0)
                    .logarithmic(true)
                    .show(ui, &mut pivot)
                    .slider;
                let c = wide_row().show(ui, &mut exposure).slider;
                let d = percent_row().show(ui, &mut margin).slider;
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
        &wide_row(),
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
            &wide_row(),
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
        &wide_row(),
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
            &wide_row(),
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
            &wide_row(),
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
        &wide_row(),
        315.,
        true,
        vec![],
        egui::Modifiers::NONE,
    );
    let start = slider.rect.left_center() + egui::vec2(40., 0.);
    frame(
        &context,
        &mut value,
        &wide_row(),
        315.,
        true,
        pointer(start, true),
        egui::Modifiers::SHIFT,
    );
    frame(
        &context,
        &mut value,
        &wide_row(),
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
        &identity_row(),
        315.,
        true,
        pointer(egui::pos2(4., 4.), false),
        egui::Modifiers::NONE,
    );
    frame(
        &context,
        &mut value,
        &wide_row(),
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
            &wide_row(),
            315.,
            true,
            vec![],
            modifiers,
        );
        let start = slider.rect.left_center() + egui::vec2(40., 0.);
        frame(
            &context,
            &mut value,
            &wide_row(),
            315.,
            true,
            pointer(start, true),
            modifiers,
        );
        let pressed = value;
        frame(
            &context,
            &mut value,
            &wide_row(),
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
            &wide_row(),
            315.,
            true,
            vec![],
            modifiers,
        );
        context.memory_mut(|m| m.request_focus(response.0.id));
        frame(
            &context,
            &mut value,
            &wide_row(),
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
        &wide_row(),
        315.,
        true,
        vec![],
        egui::Modifiers::NONE,
    );
    for pressed in [true, false] {
        frame(
            &context,
            &mut value,
            &wide_row(),
            315.,
            true,
            pointer(response.1.rect.center(), pressed),
            egui::Modifiers::NONE,
        );
    }
    frame(
        &context,
        &mut value,
        &wide_row(),
        315.,
        true,
        vec![egui::Event::Text("12.345678".into())],
        egui::Modifiers::NONE,
    );
    assert_eq!(value, 0.123456789);
    frame(
        &context,
        &mut value,
        &wide_row(),
        315.,
        true,
        vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
        egui::Modifiers::NONE,
    );
    frame(
        &context,
        &mut value,
        &wide_row(),
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
            &wide_row(),
            315.,
            true,
            pointer(response.1.rect.center(), pressed),
            egui::Modifiers::NONE,
        );
    }
    frame(
        &context,
        &mut value,
        &wide_row(),
        315.,
        true,
        vec![egui::Event::Text("-8".into())],
        egui::Modifiers::NONE,
    );
    frame(
        &context,
        &mut value,
        &wide_row(),
        315.,
        true,
        vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
        egui::Modifiers::NONE,
    );
    frame(
        &context,
        &mut value,
        &wide_row(),
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
        &wide_row(),
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
            &wide_row(),
            315.,
            true,
            vec![key(egui::Key::Tab, egui::Modifiers::NONE)],
            egui::Modifiers::NONE,
        );
    }
    frame(
        &context,
        &mut value,
        &wide_row(),
        315.,
        true,
        vec![egui::Event::Text("-11.25".into())],
        egui::Modifiers::NONE,
    );
    frame(
        &context,
        &mut value,
        &wide_row(),
        315.,
        true,
        vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
        egui::Modifiers::NONE,
    );
    frame(
        &context,
        &mut value,
        &wide_row(),
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
        &wide_row(),
        315.,
        true,
        vec![],
        egui::Modifiers::NONE,
    );
    for pressed in [true, false] {
        frame(
            &context,
            &mut value,
            &wide_row(),
            315.,
            true,
            pointer(response.1.rect.center(), pressed),
            egui::Modifiers::NONE,
        );
    }
    frame(
        &context,
        &mut value,
        &wide_row(),
        315.,
        true,
        vec![egui::Event::Text("5".into())],
        egui::Modifiers::NONE,
    );
    frame(
        &context,
        &mut value,
        &wide_row(),
        315.,
        false,
        vec![],
        egui::Modifiers::NONE,
    );
    frame(
        &context,
        &mut value,
        &wide_row(),
        315.,
        false,
        vec![],
        egui::Modifiers::NONE,
    );
    assert_eq!(value, 0.);
}

#[test]
fn parameter_reset_uses_supplied_default_and_disabled_controls_do_not_edit() {
    for enabled in [true, false] {
        let context = egui::Context::default();
        let mut value = 1.2;
        let response = frame(
            &context,
            &mut value,
            &nonzero_default_row(),
            315.,
            enabled,
            vec![],
            egui::Modifiers::NONE,
        );
        let position = response.2.rect.center();
        frame(
            &context,
            &mut value,
            &nonzero_default_row(),
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
                &nonzero_default_row(),
                315.,
                false,
                pointer(position, true),
                egui::Modifiers::NONE,
            );
            frame(
                &context,
                &mut value,
                &nonzero_default_row(),
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
fn rows_accept_supplied_endpoints_and_reset_to_identity() {
    for (initial, edge_key) in [(0.4, egui::Key::ArrowLeft), (1.6, egui::Key::ArrowRight)] {
        let context = egui::Context::default();
        let mut value = initial;
        let (slider, _, label) = frame(
            &context,
            &mut value,
            &identity_row(),
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
            &identity_row(),
            270.,
            true,
            vec![key(edge_key, egui::Modifiers::NONE)],
            egui::Modifiers::NONE,
        );
        assert_eq!(value, initial);
        frame(
            &context,
            &mut value,
            &identity_row(),
            270.,
            true,
            alt_click(label.rect.center()),
            egui::Modifiers::ALT,
        );
        assert_eq!(value, 1.);
    }
}

#[test]
fn zero_default_rows_fit_and_reset_only_when_enabled() {
    for label in ["Short", "Longer name"] {
        let row = ParameterRow::new("zero", label, 0.0..=1.0).default_value(0.);
        for width in [270., 315.] {
            for enabled in [true, false] {
                let context = egui::Context::default();
                let mut value = 0.75;
                let (_, numeric, reset) = frame(
                    &context,
                    &mut value,
                    &row,
                    width,
                    enabled,
                    vec![],
                    egui::Modifiers::NONE,
                );
                assert!(reset.rect.left() < numeric.rect.left());
                frame(
                    &context,
                    &mut value,
                    &row,
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
fn configured_precision_and_steps_preserve_values_and_reset_to_supplied_defaults() {
    for (row, default, initial, step) in [
        (
            ParameterRow::new("coarse", "Coarse", 0.0..=10.0).default_value(3.),
            3.,
            3.123456789,
            0.01,
        ),
        (
            ParameterRow::new("fine", "Fine", 0.0..=0.1)
                .default_value(0.)
                .step(0.0001)
                .decimals(4),
            0.,
            0.0123456789,
            0.0001,
        ),
    ] {
        for enabled in [true, false] {
            let context = egui::Context::default();
            let mut value = initial;
            let response = frame(
                &context,
                &mut value,
                &row,
                270.,
                enabled,
                vec![],
                egui::Modifiers::NONE,
            );
            assert_eq!(value, initial);
            frame(
                &context,
                &mut value,
                &row,
                270.,
                enabled,
                alt_click(response.2.rect.center()),
                egui::Modifiers::ALT,
            );
            assert_eq!(value, if enabled { default } else { initial });
        }
        for modifiers in [egui::Modifiers::NONE, egui::Modifiers::SHIFT] {
            let context = egui::Context::default();
            let mut value = default;
            let response = frame(&context, &mut value, &row, 315., true, vec![], modifiers);
            context.memory_mut(|m| m.request_focus(response.0.id));
            frame(
                &context,
                &mut value,
                &row,
                315.,
                true,
                vec![key(egui::Key::ArrowRight, modifiers)],
                modifiers,
            );
            let expected = default + step * if modifiers.shift { 0.1 } else { 1. };
            assert!((value - expected).abs() < 1e-12, "{value} != {expected}");
        }
    }
}
