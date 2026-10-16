use super::*;

pub(super) fn expected_crop(h: &Harness<'_, Editor>, source: SourceId, percent: f64) -> Crop {
    let p = &h.state().document.project;
    let active = p.source(source).unwrap().development.active_area.into();
    let areas = &p
        .analysis_regions
        .iter()
        .find(|a| a.source == source)
        .unwrap()
        .areas;
    crate::auto_crop::inward_margin(
        crate::auto_crop::propose(active, areas).unwrap(),
        active,
        percent,
    )
    .unwrap()
}
#[test]
fn live_slider_and_precise_numeric_drafts_preserve_the_background_and_zero_restores_originals() {
    for (name, size, width) in [
        ("normal", egui::vec2(1100., 800.), 315.),
        ("narrow", egui::vec2(800., 600.), 270.),
    ] {
        let (mut h, _directory) = setup();
        set_layout(&mut h, size, width);
        stage(&mut h, "Roll");
        click_action(&mut h, "Auto-crop roll");
        overlay::wait_context(&mut h);
        let source = h.state().current_source.unwrap();
        let before = h.state().document.project.clone();
        h.get_by_label("100%").click();
        h.step();
        overlay::wait_context(&mut h);
        let layout = h.state().last_layout.unwrap().0;
        let viewport = h.state().viewport;
        let image = h.state().working.displayed.clone().unwrap();
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
            let crop = expected_crop(&h, source, h.state().auto_crop_margin);
            assert_eq!(
                h.state().auto_crop_preview(source),
                Some(Some(crop)),
                "drag previews before release, never accepts"
            );
            assert_eq!(h.state().document.project, before);
            assert_eq!(h.state().crop, None);
            assert!(!h.state().document.dirty());
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
        assert_eq!(h.state().auto_crop_margin, 0.123456);
        let crop = expected_crop(&h, source, 0.123456);
        assert_eq!(h.state().auto_crop_preview(source), Some(Some(crop)));
        workflow::enter_numeric(&mut h, "Inward margin", "0");
        assert_eq!(
            h.state().auto_crop_preview(source),
            Some(Some(expected_crop(&h, source, 0.)))
        );
        workflow::enter_numeric(&mut h, "Inward margin", "0.123456");
        assert_eq!(
            h.state().auto_crop_preview(source),
            Some(Some(crop)),
            "no cumulative trimming"
        );
        overlay::wait_context(&mut h);
        assert_eq!(h.state().last_layout.unwrap().0.image, layout.image);
        assert_eq!(h.state().viewport, viewport);
        assert!(Arc::ptr_eq(
            &image.image,
            &h.state().working.displayed.as_ref().unwrap().image
        ));
        assert_eq!(h.state().document.project, before);
        save_capture(
            &h.render().unwrap(),
            &format!("auto-crop-margin-draft-{name}-offscreen"),
        );
        click_action(&mut h, "Auto-crop roll");
        assert!(
            h.state().auto_crop_edit.is_none(),
            "clicking the active shelf tool cancels its draft"
        );
        assert_eq!(h.state().document.project, before);
        assert!(
            h.query_by_role_and_label(egui::accesskit::Role::Slider, "Inward margin")
                .is_none()
        );
    }
}
