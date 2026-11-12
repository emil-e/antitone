use super::*;
use crate::editor::ui::{configure_branding, test_support::key, theme::INSCRIPTION};
use egui::{Context, Pos2, Rect};

#[test]
fn brand_assets_have_the_expected_sizes_and_self_contained_wordmarks() {
    let header = image::load_from_memory(include_bytes!(
        "../../../../assets/branding/wordmark-header.png"
    ))
    .unwrap()
    .into_rgba8();
    assert_eq!(header.dimensions(), (472, 64));
    assert!(
        header
            .pixels()
            .any(|pixel| pixel.0 == INSCRIPTION.to_array())
    );
    assert!(header.pixels().any(|pixel| pixel.0[3] == 0));
    for svg in [
        include_str!("../../../../assets/branding/wordmark-amber.svg"),
        include_str!("../../../../assets/branding/wordmark-density.svg"),
    ] {
        assert!(svg.contains("<path"));
        assert!(!svg.contains("<text"));
        assert!(!svg.contains("href="));
    }
    let ico = include_bytes!("../../../../assets/branding/icon.ico");
    assert_eq!(&ico[..6], &[0, 0, 1, 0, 7, 0]);
    let mut next_offset = 6 + 7 * 16;
    for (index, size) in [16, 24, 32, 48, 64, 128, 256].into_iter().enumerate() {
        let entry = &ico[6 + index * 16..6 + (index + 1) * 16];
        assert_eq!(entry[0] as u32, size % 256);
        assert_eq!(entry[1], entry[0]);
        let length = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
        let offset = u32::from_le_bytes(entry[12..16].try_into().unwrap()) as usize;
        assert_eq!(offset, next_offset);
        let image = image::load_from_memory(&ico[offset..offset + length]).unwrap();
        assert_eq!((image.width(), image.height()), (size, size));
        next_offset += length;
    }
    assert_eq!(next_offset, ico.len());
    let icns = include_bytes!("../../../../assets/branding/icon.icns");
    assert_eq!(&icns[..4], b"icns");
    assert_eq!(
        u32::from_be_bytes(icns[4..8].try_into().unwrap()) as usize,
        icns.len()
    );
}

#[test]
fn app_header_keeps_branding_and_menus_visible_without_taking_keyboard_focus() {
    for scale in [1., 2.] {
        for width in [320., 720., 1100.] {
            let context = Context::default();
            context.enable_accesskit();
            let mut buttons = None;
            let mut draw = |events| {
                let mut input = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(width, 200.))),
                    events,
                    ..Default::default()
                };
                input
                    .viewports
                    .get_mut(&egui::ViewportId::ROOT)
                    .unwrap()
                    .native_pixels_per_point = Some(scale);
                let mut output = context.run_ui(input, |ui| {
                    configure_branding(ui);
                    assert_eq!(ui.ctx().pixels_per_point(), scale);
                    assert_eq!(ui.visuals().selection.stroke.color, INSCRIPTION);
                    let surface = ui.visuals().panel_fill;
                    assert!(surface.r().abs_diff(surface.g()) <= 4);
                    assert!(surface.g().abs_diff(surface.b()) <= 4);
                    app_header(ui, |ui| {
                        let file = ui.menu_button("File", |_| ());
                        let export = ui.menu_button("Export", |_| ());
                        let name = ui.add(
                            egui::Label::new("A very long project filename.antitone.json")
                                .truncate(),
                        );
                        assert!(
                            name.rect.right() <= width,
                            "name {:?}, screen {width}, scale {scale}",
                            name.rect
                        );
                        buttons = Some([file.response, export.response]);
                    });
                });
                output.textures_delta.clear();
                output
            };
            let output = draw(vec![]);
            let update = output.platform_output.accesskit_update.as_ref().unwrap();
            let mark = &update
                .nodes
                .iter()
                .find(|(_, node)| node.value() == Some("Antitone"))
                .unwrap()
                .1;
            assert_eq!(mark.role(), egui::accesskit::Role::Label);
            let bounds = mark.bounds().unwrap();
            assert!(bounds.width() > 0. && bounds.height() > 0.);
            let artwork = image::load_from_memory(include_bytes!(
                "../../../../assets/branding/wordmark-header.png"
            ))
            .unwrap();
            let aspect = f64::from(artwork.width()) / f64::from(artwork.height());
            assert!((bounds.width() / bounds.height() - aspect).abs() < 1e-5);
            assert!(bounds.x0 >= 0. && bounds.x1 <= f64::from(width));
            assert!(bounds.y0 >= 0. && bounds.y1 <= 200.);
            let mark_before = bounds;
            let after = draw(key(egui::Key::Tab));
            let mark_after = after
                .platform_output
                .accesskit_update
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .find(|(_, node)| node.value() == Some("Antitone"))
                .map(|(_, node)| node.bounds().unwrap());
            if let Some(bounds) = mark_after {
                assert_eq!(bounds, mark_before);
            }
            drop(draw);
            let buttons = buttons.unwrap();
            assert_eq!(
                context.memory(|memory| memory.focused()),
                Some(buttons[0].id)
            );
            let viewport = Rect::from_min_size(Pos2::ZERO, egui::vec2(width, 200.));
            assert!(
                buttons
                    .iter()
                    .all(|button| viewport.contains_rect(button.rect))
            );
            assert!(f64::from(buttons[0].rect.left()) >= mark_before.x1);
            assert!(!buttons[0].rect.intersects(buttons[1].rect));
        }
    }
}
