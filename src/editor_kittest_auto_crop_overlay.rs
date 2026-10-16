use super::{margin::expected_crop, *};

pub(super) fn wait_context(h: &mut Harness<'_, Editor>) {
    wait_for(h, |e| {
        assert!(e.auto_crop_context());
        assert!(e.working.error.is_none(), "{:?}", e.working.error);
        let Some(ready) = &e.ready else {
            return false;
        };
        !e.working.updating
            && e.working.displayed.as_ref().is_some_and(|v| {
                v.spec.authority == e.id
                    && v.spec.frame.composition.crop() == Crop::from_region(ready.area)
            })
    });
    let timing = Arc::new(SelectionTiming::new(h.state().id, Instant::now()));
    h.state_mut().selection_timing = Some(timing.clone());
    h.step();
    h.render().unwrap();
    assert!(timing.ready_was_painted());
    let e = h.state();
    let layout = e.last_layout.unwrap().0;
    assert_eq!(
        layout.composition.crop(),
        Crop::from_region(e.ready.as_ref().unwrap().area)
    );
    match e.scope_request().unwrap() {
        scopes::ScopeInput::Rendered(view, frame) => {
            assert!(Arc::ptr_eq(&view, e.working.displayed.as_ref().unwrap()));
            assert_eq!(frame.composition, layout.composition);
        }
        _ => panic!(
            "representative view and scopes share the full physical scan, not artwork or an accepted crop"
        ),
    }
}
fn pointer(h: &mut Harness<'_, Editor>, at: egui::Pos2, pressed: bool) {
    h.hover_at(at);
    h.event(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
}
#[test]
fn representative_batch_preview_reuses_the_image_browses_freely_and_commits_only_on_apply() {
    for (name, size, width) in [
        ("normal", egui::vec2(1100., 800.), 315.),
        ("narrow", egui::vec2(800., 600.), 270.),
    ] {
        let (mut h, directory) = setup();
        set_layout(&mut h, size, width);
        stage(&mut h, "Photo");
        h.get_by_label("Original-resolution reference").click();
        h.step();
        workflow::wait_settled(&mut h);
        let ids = h.state().document.project.order.clone();
        let before = h.state().document.project.clone();
        h.get_by_label("crop-2.dng")
            .click_modifiers(egui::Modifiers::COMMAND);
        h.step();
        h.get_by_label("crop-2.dng").click_secondary();
        h.step();
        h.step();
        h.get_by_label("Auto-crop selected photos").click();
        h.step();
        h.step();
        assert_eq!(
            h.state().current_source,
            Some(ids[0]),
            "no mandatory browsing or per-photo review"
        );
        wait_context(&mut h);
        workflow::wait_settled(&mut h);
        assert_lifecycle(&h, size);
        let layout = h.state().last_layout.unwrap().0;
        let outline = h.state().auto_crop_outline(layout).unwrap();
        assert!(outline.iter().all(|p| layout.image.contains(*p)));
        assert!(egui::Rect::from_points(&outline).width() < layout.image.width());
        let viewport = h.state().viewport;
        let image = h.state().working.displayed.clone().unwrap();
        let revision = h.state().original.revision();
        workflow::enter_numeric(&mut h, "Inward margin", "1.25");
        wait_context(&mut h);
        assert_ne!(h.state().auto_crop_outline(layout).unwrap(), outline);
        let q = h.state().auto_crop_outline(layout).unwrap();
        let start = q[0];
        let end =
            start + ((q[1] - q[0]).normalized() + (q[3] - q[0]).normalized()) * layout.scale * 6.;
        pointer(&mut h, start, true);
        h.hover_at(end);
        h.step();
        assert!(h.state().auto_crop_margin > 1.25);
        pointer(&mut h, end, false);
        wait_context(&mut h);
        assert_eq!(h.state().document.project, before);
        assert!(!h.state().document.dirty());
        assert_eq!(h.state().crop, None);
        assert_eq!(h.state().viewport, viewport);
        assert_eq!(h.state().last_layout.unwrap().0.image, layout.image);
        assert_eq!(h.state().original.revision(), revision);
        assert!(Arc::ptr_eq(
            &image.image,
            &h.state().working.displayed.as_ref().unwrap().image
        ));
        save_capture(
            &h.render().unwrap(),
            &format!("auto-crop-batch-{name}-offscreen"),
        );
        let saved = h.state().document.path.clone().unwrap();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
        h.step();
        assert!(h.state().document_pending.is_none());
        assert_eq!(crate::project::Project::load(&saved).unwrap(), before);
        let output = directory.path().join("draft.tiff");
        h.state_mut().export(
            output.clone(),
            OutputFormat::Tiff(OutputTarget::Srgb),
            Overwrite::Refuse,
        );
        h.step();
        assert!(
            !output.exists() && h.state().export_pending.is_none() && !h.state().busy,
            "output commands cannot export a draft"
        );
        for index in [2, 1] {
            h.get_by_label(&format!("crop-{index}.dng")).click();
            h.step();
            wait_for(&mut h, |e| !e.busy && e.ready.is_some());
            wait_context(&mut h);
            assert_eq!(h.state().current_source, Some(ids[index]));
            assert_eq!(h.state().document.project, before);
            assert_eq!(
                h.state()
                    .auto_crop_outline(h.state().last_layout.unwrap().0)
                    .is_some(),
                index == 2
            );
        }
        assert!(
            h.query_by_label("Tool warning").is_none(),
            "outside the captured set is not a metadata failure"
        );
        let request = h.state().id;
        workflow::enter_numeric(&mut h, "Inward margin", "2");
        assert_eq!(h.state().id, request);
        assert!(h.state().auto_crop_preview(ids[1]).is_none());
        assert_eq!(h.state().document.project, before);
        let mut expected = before;
        for id in [ids[0], ids[2]] {
            let crop = expected_crop(&h, id, 2.);
            assert_eq!(h.state().auto_crop_preview(id), Some(Some(crop)));
            expected.source_mut(id).unwrap().crop = Some(crop);
        }
        click_action(&mut h, "Apply");
        assert_eq!(h.state().document.project, expected);
        assert_eq!(h.state().current_source, Some(ids[1]));
        assert_eq!(h.state().crop, None);
        let full = Crop::from_region(h.state().ready.as_ref().unwrap().area);
        wait_crop(&mut h, full);
        save_existing_project(&mut h);
        assert_eq!(crate::project::Project::load(&saved).unwrap(), expected);
        h.get_by_label("crop-0.dng").click();
        h.step();
        wait_for(&mut h, |e| !e.busy && e.ready.is_some());
        let crop = expected.source(ids[0]).unwrap().crop.unwrap();
        wait_crop(&mut h, crop);
        let composition = Composition::with_crop(
            h.state().ready.as_ref().unwrap().area,
            h.state().roll_orientation,
            h.state().photo_orientation,
            Some(crop),
        )
        .unwrap();
        h.state_mut().export(
            output.clone(),
            OutputFormat::Tiff(OutputTarget::Srgb),
            Overwrite::Refuse,
        );
        h.step();
        wait_for(&mut h, |e| !e.busy);
        assert!(h.state().failure.is_none(), "{}", h.state().status);
        let mut decoder =
            tiff::decoder::Decoder::new(std::fs::File::open(output).unwrap()).unwrap();
        let [w, height] = composition.size();
        assert_eq!(decoder.dimensions().unwrap(), (w, height));
        assert_eq!(h.state().document.project, expected);
    }
}
#[test]
fn numeric_popup_disabled_and_hidden_artwork_input_cannot_accidentally_accept_or_edit_a_draft() {
    let (mut h, _directory) = setup();
    stage(&mut h, "Roll");
    click_action(&mut h, "Auto-crop roll");
    wait_context(&mut h);
    let before = h.state().document.project.clone();
    let source = h.state().current_source.unwrap();
    let initial = h.state().auto_crop_preview(source);
    h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Inward margin")
        .click();
    h.step();
    h.event(egui::Event::Text("4".into()));
    h.step();
    h.key_press(egui::Key::Escape);
    h.step();
    h.step();
    assert!(h.state().auto_crop_edit.is_some());
    assert_eq!(h.state().auto_crop_margin, 0.5);
    assert_eq!(h.state().auto_crop_preview(source), initial);
    h.get_by_label("Viewer help").click();
    h.step();
    assert!(egui::Popup::is_any_open(&h.ctx));
    h.key_press(egui::Key::Escape);
    h.step();
    h.step();
    assert!(!egui::Popup::is_any_open(&h.ctx));
    assert!(h.state().auto_crop_edit.is_some());
    let layout = h.state().last_layout.unwrap().0;
    let q = h.state().auto_crop_outline(layout).unwrap();
    let end = q[0] + (q[2] - q[0]) * 0.1;
    for checkbox in [false, true] {
        if checkbox {
            h.get_by_label("Show crop outline").click();
            h.step();
        } else {
            h.event(egui::Event::Key {
                key: egui::Key::H,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
            h.step();
        }
        drag(&mut h, q[0], end);
        assert!(h.state().auto_crop_context());
        assert_eq!(h.state().auto_crop_margin, 0.5);
        assert_eq!(h.state().auto_crop_preview(source), initial);
        if checkbox {
            h.get_by_label("Show crop outline").click();
            h.step();
        } else {
            h.event(egui::Event::Key {
                key: egui::Key::H,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
            h.step();
        }
    }
    h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Inward margin")
        .click();
    h.step();
    h.event(egui::Event::Text("3.5".into()));
    h.step();
    h.state_mut().busy = true;
    h.step();
    h.step();
    h.state_mut().busy = false;
    h.step();
    h.step();
    assert_eq!(h.state().auto_crop_margin, 0.5);
    assert_eq!(h.state().auto_crop_preview(source), initial);
    assert_eq!(h.state().document.project, before);
    h.key_press(egui::Key::Delete);
    h.step();
    assert!(h.state().removal_pending.is_none());
    h.key_press(egui::Key::Escape);
    h.step();
    h.step();
    assert!(h.state().auto_crop_edit.is_none());
    assert_eq!(h.state().document.project, before);
    assert!(!h.state().document.dirty());
}
