use super::*;

pub(super) fn expected_crop(h: &Harness<'_, Editor>, source: SourceId, percent: f64) -> Crop {
    h.read(|e| {
        let p = &e.document.project;
        let active = p.source(source).unwrap().development.active_area.into();
        // This is the UI/adaptor consistency oracle, not independent border truth. Pure
        // geometry tests separately check source conversion, containment and rounding.
        let metadata = e.document.spatial.ready();
        let regions = metadata
            .groups
            .iter()
            .flat_map(|group| &group.sources)
            .find(|row| row.input.source == source)
            .unwrap();
        crate::auto_crop::inward_margin(
            crate::auto_crop::propose_spatial(regions).unwrap(),
            active,
            percent,
        )
        .unwrap()
    })
}

#[test]
fn live_slider_and_precise_numeric_drafts_preserve_the_background_and_zero_restores_originals() {
    for (name, size, width) in [
        ("normal", egui::vec2(1100., 800.), 315.),
        ("narrow", egui::vec2(800., 600.), 270.),
    ] {
        setup(|mut h, _directory| {
            set_layout(&mut h, size, width);
            stage(&mut h, "Roll");
            click_action(&mut h, "Auto-crop roll");
            overlay::wait_context(&mut h);
            let source = h.read(|editor| editor.current_source.unwrap());
            let before = h.read(|editor| editor.document.project.clone());
            h.get_by_label("100%").click();
            h.step();
            overlay::wait_context(&mut h);
            let layout = h.read(|editor| editor.last_layout.unwrap().0);
            let viewport = h.read(|editor| editor.viewport);
            let image = h.read(|editor| editor.working.displayed.clone().unwrap());
            h.get_by_role_and_label(egui::accesskit::Role::Slider, "Inward margin")
                .scroll_to_me();
            h.try_run_realtime().unwrap();
            let rect = h
                .get_by_role_and_label(egui::accesskit::Role::Slider, "Inward margin")
                .rect();
            let at = |fraction| egui::pos2(rect.left() + rect.width() * fraction, rect.center().y);
            h.hover_at(at(0.2));
            h.event(egui::Event::PointerButton {
                pos: at(0.2),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            });
            h.step();
            let mut seen = Vec::new();
            for fraction in [0.35, 0.55, 0.3] {
                h.hover_at(at(fraction));
                h.step();
                let crop = h.read(|editor| expected_crop(&h, source, editor.auto_crop_margin));
                h.read(|editor| {
                    assert_eq!(
                        editor.auto_crop_preview(source),
                        Some(Some(crop)),
                        "drag previews before release, never accepts"
                    )
                });
                h.read(|editor| assert_eq!(editor.document.project, before));
                h.read(|editor| assert_eq!(editor.crop, None));
                h.read(|editor| assert!(!editor.document.dirty()));
                seen.push(crop);
            }
            assert!(seen.windows(2).all(|w| w[0] != w[1]));
            h.event(egui::Event::PointerButton {
                pos: at(0.3),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
            h.step();
            workflow::enter_numeric(&mut h, "Inward margin", "0.123456");
            overlay::wait_context(&mut h);
            h.read(|editor| assert_eq!(editor.auto_crop_margin, 0.123456));
            let crop = expected_crop(&h, source, 0.123456);
            h.read(|editor| assert_eq!(editor.auto_crop_preview(source), Some(Some(crop))));
            workflow::enter_numeric(&mut h, "Inward margin", "0");
            h.read(|editor| {
                assert_eq!(
                    editor.auto_crop_preview(source),
                    Some(Some(expected_crop(&h, source, 0.)))
                )
            });
            workflow::enter_numeric(&mut h, "Inward margin", "0.123456");
            h.read(|editor| {
                assert_eq!(
                    editor.auto_crop_preview(source),
                    Some(Some(crop)),
                    "no cumulative trimming"
                )
            });
            overlay::wait_context(&mut h);
            h.read(|editor| assert_eq!(editor.last_layout.unwrap().0.image, layout.image));
            h.read(|editor| assert_eq!(editor.viewport, viewport));
            h.read(|editor| {
                assert!(Arc::ptr_eq(
                    &image.image,
                    &editor.working.displayed.as_ref().unwrap().image
                ))
            });
            h.read(|editor| assert_eq!(editor.document.project, before));
            save_capture(
                &h.render().unwrap(),
                &format!("auto-crop-margin-draft-{name}-offscreen"),
            );
            click_action(&mut h, "Auto-crop roll");
            h.read(|editor| {
                assert!(
                    editor.document.auto_crop_edit.is_none(),
                    "clicking the active shelf tool cancels its draft"
                )
            });
            h.read(|editor| assert_eq!(editor.document.project, before));
            assert!(
                h.query_by_role_and_label(egui::accesskit::Role::Slider, "Inward margin")
                    .is_none()
            );
        });
    }
}
