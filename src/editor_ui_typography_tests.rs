use super::super::*;

#[test]
fn typography_preserves_fallbacks_tabular_digits_and_separate_reading_and_instrument_roles() {
    for scale in [1., 2.] {
        let context = egui::Context::default();
        configure_fonts(&context);
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(315., 240.),
            )),
            ..Default::default()
        };
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .native_pixels_per_point = Some(scale);
        let mut output = context.run_ui(input, |ui| {
            ui.fonts_mut(|fonts| {
                let defaults = egui::FontDefinitions::default();
                let definitions = fonts.definitions();
                let reading = &definitions.families[&egui::FontFamily::Proportional];
                assert_eq!(reading[0], READING_FONT);
                assert_eq!(
                    &reading[1..],
                    &defaults.families[&egui::FontFamily::Proportional]
                );
                assert_eq!(
                    definitions.families[&egui::FontFamily::Monospace],
                    defaults.families[&egui::FontFamily::Monospace]
                );
                let instrument = &definitions.families[&instrument_font(14.).family];
                assert_eq!(instrument[0], INSTRUMENT_FONT);
                assert_eq!(&instrument[1..], reading);
                for font in [egui::FontId::proportional(14.), instrument_font(14.)] {
                    let widths: Vec<_> = (0..10)
                        .map(|digit| {
                            fonts
                                .layout_no_wrap(
                                    digit.to_string(),
                                    font.clone(),
                                    egui::Color32::WHITE,
                                )
                                .size()
                                .x
                        })
                        .collect();
                    assert!(widths.iter().all(|width| (width - widths[0]).abs() < 1e-5));
                }
            });
            ui.label("0.123456");
            ui.strong("Reading heading");
            ui.label(instrument_label("Gamma neutral", 12.));
            ui.label(instrument_readout("Source +0.00 EV", 10.));
            for enabled in [true, false] {
                for minimum in [0., 50.] {
                    let action = TextAction::new("1234567890%")
                        .readout()
                        .toolbar(minimum)
                        .enabled(enabled);
                    let measured = action.width(ui);
                    assert!(
                        measured > minimum,
                        "fixture must exercise font measurement, not only minimum sizing"
                    );
                    let response = action.show(ui);
                    assert_eq!(response.enabled(), enabled);
                    assert!((response.rect.width() - measured).abs() < 1e-5);
                    assert_eq!(response.rect.height(), CONTROL_HEIGHT);
                }
            }
        });
        output.textures_delta.clear();
        for (text, instrument, tracked) in [
            ("0.123456", false, false),
            ("Reading heading", false, false),
            ("Gamma neutral", true, true),
            ("Source +0.00 EV", true, false),
            ("1234567890%", true, false),
        ] {
            let shape = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(shape)
                        if shape.galley.job.text.eq_ignore_ascii_case(text) =>
                    {
                        Some(shape)
                    }
                    _ => None,
                })
                .unwrap();
            for section in &shape.galley.job.sections {
                assert_eq!(
                    section.format.font_id.family,
                    if instrument {
                        instrument_font(14.).family
                    } else {
                        egui::FontFamily::Proportional
                    }
                );
                if tracked {
                    assert!(section.format.extra_letter_spacing > 0.);
                } else {
                    assert_eq!(section.format.extra_letter_spacing, 0.);
                }
            }
        }
    }
}
