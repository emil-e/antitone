use super::{margin::expected_crop, *};

pub(super) fn wait_context(h: &mut Harness<'_, Editor>) {
    wait_for(h, |e| {
        if e.document.spatial_preparing.is_some() {
            return false;
        }
        assert!(e.auto_crop_context());
        assert!(
            e.document.visit.working_view.error.is_none(),
            "{:?}",
            e.document.visit.working_view.error
        );
        let Some(ready) = &e.document.visit.canvas.ready else {
            return false;
        };
        !e.document.visit.working_view.updating
            && e.document
                .visit
                .working_view
                .displayed
                .as_ref()
                .is_some_and(|v| {
                    v.spec.authority == e.id
                        && v.spec.frame().composition.crop() == Crop::from_region(ready.area)
                })
    });
    let timing = h.read(|editor| Arc::new(SelectionTiming::new(editor.id, Instant::now())));
    h.update(|editor| editor.document.visit.canvas.selection_timing = Some(timing.clone()));
    h.step();
    h.render().unwrap();
    assert!(timing.ready_was_painted());
    h.read(|e| {
    let layout = e.active_canvas().last_layout.unwrap().0;
    assert_eq!(
        layout.composition.crop(),
        Crop::from_region(e.document.visit.canvas.ready.as_ref().unwrap().area)
    );
    match e.scope_request().unwrap() {
        scopes::ScopeInput::Rendered(view, frame) => {
            assert!(Arc::ptr_eq(&view, e.document.visit.working_view.displayed.as_ref().unwrap()));
            assert_eq!(frame.composition, layout.composition);
        }
        _ => panic!(
            "representative view and scopes share the full physical scan, not artwork or an accepted crop"
        ),
    }
    });
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
        setup(|mut h, directory| {
            set_layout(&mut h, size, width);
            stage(&mut h, "Photo");
            h.get_by_label("Original-resolution reference").click();
            h.step();
            workflow::wait_settled(&mut h);
            let ids = h.read(|editor| editor.document.state.project.order.clone());
            let before = h.read(|editor| editor.document.state.project.clone());
            h.get_by_label("crop-2.dng")
                .click_modifiers(egui::Modifiers::COMMAND);
            h.step();
            h.get_by_label("crop-2.dng").click_secondary();
            h.step();
            h.step();
            h.get_by_label("Auto-crop selected photos").click();
            h.step();
            h.step();
            h.read(|editor| {
                assert_eq!(
                    editor.document.visit.current_source,
                    Some(ids[0]),
                    "no mandatory browsing or per-photo review"
                )
            });
            wait_context(&mut h);
            workflow::wait_settled(&mut h);
            assert_lifecycle(&h, size);
            let layout = h.read(|editor| editor.active_canvas().last_layout.unwrap().0);
            let outline = h.read(|editor| editor.auto_crop_outline(layout).unwrap());
            assert!(outline.iter().all(|p| layout.image.contains(*p)));
            assert!(egui::Rect::from_points(&outline).width() < layout.image.width());
            let viewport = h.read(|editor| editor.document.visit.canvas.viewport);
            let image = h.read(|editor| {
                editor
                    .document
                    .visit
                    .working_view
                    .displayed
                    .clone()
                    .unwrap()
            });
            let revision = h.read(|editor| editor.document.visit.canvas.grade.original.revision());
            workflow::enter_numeric(&mut h, "Margin", "1.25");
            wait_context(&mut h);
            h.read(|editor| assert_ne!(editor.auto_crop_outline(layout).unwrap(), outline));
            let q = h.read(|editor| editor.auto_crop_outline(layout).unwrap());
            let start = q[0];
            let end = start
                + ((q[1] - q[0]).normalized() + (q[3] - q[0]).normalized()) * layout.scale * 6.;
            pointer(&mut h, start, true);
            h.hover_at(end);
            h.step();
            h.read(|editor| assert!(editor.auto_crop_margin > 1.25));
            pointer(&mut h, end, false);
            wait_context(&mut h);
            h.read(|editor| assert_eq!(editor.document.state.project, before));
            h.read(|editor| assert!(!editor.document.dirty()));
            h.read(|editor| assert_eq!(editor.document.visit.crop, None));
            h.read(|editor| assert_eq!(editor.document.visit.canvas.viewport, viewport));
            h.read(|editor| {
                assert_eq!(
                    editor.active_canvas().last_layout.unwrap().0.image,
                    layout.image
                )
            });
            h.read(|editor| {
                assert_eq!(
                    editor.document.visit.canvas.grade.original.revision(),
                    revision
                )
            });
            h.read(|editor| {
                assert!(Arc::ptr_eq(
                    &image.image,
                    &editor
                        .document
                        .visit
                        .working_view
                        .displayed
                        .as_ref()
                        .unwrap()
                        .image
                ))
            });
            save_capture(
                &h.render().unwrap(),
                &format!("auto-crop-batch-{name}-offscreen"),
            );
            let saved = h.read(|editor| editor.document.path.clone().unwrap());
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
            h.step();
            h.read(|editor| assert!(editor.document_pending.is_none()));
            let (accepted, regions) =
                crate::project::Project::load_with_spatial_metadata(&saved).unwrap();
            assert_eq!(accepted, before);
            assert_eq!(
                regions,
                h.read(|editor| editor.document.capture().ready_spatial_metadata())
            );
            let output = directory.path().join("draft.tiff");
            h.enter(|editor, cx| {
                editor.export(
                    output.clone(),
                    OutputFormat::Tiff(OutputTarget::Srgb),
                    Overwrite::Refuse,
                    cx,
                )
            });
            h.step();
            h.read(|editor| {
                assert!(
                    !output.exists()
                        && editor.document.export_operation.pending.is_none()
                        && !editor.busy,
                    "output commands cannot export a draft"
                )
            });
            for index in [2, 1] {
                h.get_by_label(&format!("crop-{index}.dng")).click();
                h.step();
                wait_for(&mut h, |e| {
                    !e.busy && e.document.visit.canvas.ready.is_some()
                });
                wait_context(&mut h);
                h.read(|editor| assert_eq!(editor.document.visit.current_source, Some(ids[index])));
                h.read(|editor| assert_eq!(editor.document.state.project, before));
                h.read(|editor| {
                    assert_eq!(
                        editor
                            .auto_crop_outline(editor.active_canvas().last_layout.unwrap().0)
                            .is_some(),
                        index == 2
                    )
                });
            }
            assert!(
                h.query_by_label("Tool warning").is_none(),
                "outside the captured set is not a metadata failure"
            );
            let request = h.read(|editor| editor.id);
            workflow::enter_numeric(&mut h, "Margin", "2");
            h.read(|editor| assert_eq!(editor.id, request));
            h.read(|editor| assert!(editor.auto_crop_preview(ids[1]).is_none()));
            h.read(|editor| assert_eq!(editor.document.state.project, before));
            let mut expected = before;
            for id in [ids[0], ids[2]] {
                let crop = expected_crop(&h, id, 2.);
                h.read(|editor| assert_eq!(editor.auto_crop_preview(id), Some(Some(crop))));
                expected.source_mut(id).unwrap().crop = Some(crop);
            }
            click_action(&mut h, "Apply");
            h.read(|editor| assert_eq!(editor.document.state.project, expected));
            h.read(|editor| assert_eq!(editor.document.visit.current_source, Some(ids[1])));
            h.read(|editor| assert_eq!(editor.document.visit.crop, None));
            let full = h.read(|editor| {
                Crop::from_region(editor.document.visit.canvas.ready.as_ref().unwrap().area)
            });
            wait_crop(&mut h, full);
            save_existing_project(&mut h);
            let (accepted, regions) =
                crate::project::Project::load_with_spatial_metadata(&saved).unwrap();
            assert_eq!(accepted, expected);
            assert_eq!(
                regions,
                h.read(|editor| editor.document.capture().ready_spatial_metadata())
            );
            h.get_by_label("crop-0.dng").click();
            h.step();
            wait_for(&mut h, |e| {
                !e.busy && e.document.visit.canvas.ready.is_some()
            });
            let crop = expected.source(ids[0]).unwrap().crop.unwrap();
            wait_crop(&mut h, crop);
            let composition = h.read(|editor| {
                Composition::with_crop(
                    editor.document.visit.canvas.ready.as_ref().unwrap().area,
                    editor.document.roll_orientation,
                    editor.document.visit.photo_orientation,
                    Some(crop),
                )
                .unwrap()
            });
            h.enter(|editor, cx| {
                editor.export(
                    output.clone(),
                    OutputFormat::Tiff(OutputTarget::Srgb),
                    Overwrite::Refuse,
                    cx,
                )
            });
            h.step();
            wait_for(&mut h, |e| !e.busy);
            h.read(|editor| assert!(editor.failure.is_none(), "{}", editor.status));
            let mut decoder =
                tiff::decoder::Decoder::new(std::fs::File::open(output).unwrap()).unwrap();
            let [w, height] = composition.size();
            assert_eq!(decoder.dimensions().unwrap(), (w, height));
            h.read(|editor| assert_eq!(editor.document.state.project, expected));
        });
    }
}

#[test]
fn numeric_popup_disabled_and_hidden_artwork_input_cannot_accidentally_accept_or_edit_a_draft() {
    setup(|mut h, _directory| {
        stage(&mut h, "Roll");
        click_action(&mut h, "Auto-crop roll");
        wait_context(&mut h);
        let before = h.read(|editor| editor.document.state.project.clone());
        let source = h.read(|editor| editor.document.visit.current_source.unwrap());
        let initial = h.read(|editor| editor.auto_crop_preview(source));
        h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Margin")
            .click();
        h.step();
        h.event(egui::Event::Text("4".into()));
        h.step();
        h.key_press(egui::Key::Escape);
        h.step();
        h.step();
        h.read(|editor| assert!(editor.document.auto_crop_edit().is_some()));
        h.read(|editor| assert_eq!(editor.auto_crop_margin, 0.5));
        h.read(|editor| assert_eq!(editor.auto_crop_preview(source), initial));
        h.get_by_label("Viewer help").click();
        h.step();
        assert!(egui::Popup::is_any_open(&h.ctx));
        h.key_press(egui::Key::Escape);
        h.step();
        h.step();
        assert!(!egui::Popup::is_any_open(&h.ctx));
        h.read(|editor| assert!(editor.document.auto_crop_edit().is_some()));
        let layout = h.read(|editor| editor.active_canvas().last_layout.unwrap().0);
        let q = h.read(|editor| editor.auto_crop_outline(layout).unwrap());
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
            h.read(|editor| assert!(editor.auto_crop_context()));
            h.read(|editor| assert_eq!(editor.auto_crop_margin, 0.5));
            h.read(|editor| assert_eq!(editor.auto_crop_preview(source), initial));
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
        h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Margin")
            .click();
        h.step();
        h.event(egui::Event::Text("3.5".into()));
        h.step();
        h.update(|editor| editor.busy = true);
        h.step();
        h.step();
        h.update(|editor| editor.busy = false);
        h.step();
        h.step();
        h.read(|editor| assert_eq!(editor.auto_crop_margin, 0.5));
        h.read(|editor| assert_eq!(editor.auto_crop_preview(source), initial));
        h.read(|editor| assert_eq!(editor.document.state.project, before));
        h.key_press(egui::Key::Delete);
        h.step();
        h.read(|editor| assert!(editor.document.removal_pending.is_none()));
        h.key_press(egui::Key::Escape);
        h.step();
        h.step();
        h.read(|editor| assert!(editor.document.auto_crop_edit().is_none()));
        h.read(|editor| assert_eq!(editor.document.state.project, before));
        h.read(|editor| assert!(!editor.document.dirty()));
    });
}
