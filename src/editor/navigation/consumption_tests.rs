use super::*;

#[derive(Clone, Copy, Debug)]
enum Gesture {
    Click,
    Drag,
    SpaceDrag,
    MiddleDrag,
    Zoom,
}

#[test]
fn consumed_primary_does_not_navigate_but_explicit_pan_zoom_and_owner_release_survive() {
    for gesture in [
        Gesture::Click,
        Gesture::Drag,
        Gesture::SpaceDrag,
        Gesture::MiddleDrag,
        Gesture::Zoom,
    ] {
        for consumed in [false, true] {
            let context = egui::Context::default();
            let rect = egui::Rect::from_min_size(egui::pos2(10., 10.), egui::vec2(200., 150.));
            let composition = Composition::new(
                Rect {
                    x: 0,
                    y: 0,
                    width: 800,
                    height: 600,
                },
                Default::default(),
                Default::default(),
            )
            .unwrap();
            let mut viewport = Viewport::default();
            if !matches!(gesture, Gesture::Click) {
                viewport.zoom = Some(1.);
            }
            let mut owner_click = false;
            let mut owner_drag = false;
            let mut owner_release = false;
            let mut render = |events| {
                let mut output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(400., 300.),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        let original = ui.interact(
                            rect,
                            ui.id().with("gesture"),
                            egui::Sense::click_and_drag(),
                        );
                        let mut delegated = original.clone();
                        if consumed {
                            consume_primary(&mut delegated);
                        }
                        owner_click |= original.clicked_by(egui::PointerButton::Primary);
                        owner_drag |= original.dragged_by(egui::PointerButton::Primary);
                        owner_release |= original.drag_stopped_by(egui::PointerButton::Primary);
                        let relayout =
                            |v: &mut Viewport| v.layout_composition(rect, composition, 1.);
                        let layout = relayout(&mut viewport);
                        navigate_view(&mut viewport, ui, &delegated, layout, relayout);
                    },
                );
                output.textures_delta.clear();
            };
            render(vec![]);
            render(vec![egui::Event::PointerMoved(rect.center())]);
            let middle = matches!(gesture, Gesture::MiddleDrag);
            let space = matches!(gesture, Gesture::SpaceDrag);
            let button = if middle {
                egui::PointerButton::Middle
            } else {
                egui::PointerButton::Primary
            };
            let start = rect.center();
            let end = start + egui::vec2(35., 20.);
            if matches!(gesture, Gesture::Zoom) {
                render(vec![egui::Event::Zoom(2.)]);
            } else {
                let mut press = vec![egui::Event::PointerButton {
                    pos: start,
                    button,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                }];
                if space {
                    press.push(egui::Event::Key {
                        key: egui::Key::Space,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
                render(press);
                if !matches!(gesture, Gesture::Click) {
                    render(vec![egui::Event::PointerMoved(end)]);
                }
                render(vec![egui::Event::PointerButton {
                    pos: if matches!(gesture, Gesture::Click) {
                        start
                    } else {
                        end
                    },
                    button,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }]);
            }
            match gesture {
                Gesture::Click => {
                    assert!(
                        owner_click,
                        "native click positive control: consumed={consumed}"
                    );
                    assert_eq!(viewport.zoom, if consumed { None } else { Some(1.) });
                }
                Gesture::Zoom => assert_eq!(viewport.zoom, Some(2.)),
                _ => {
                    let layout = viewport.layout_composition(rect, composition, 1.);
                    let center = layout.source_point(rect.center());
                    let moved = !matches!(gesture, Gesture::Drag) || !consumed;
                    assert_eq!(
                        center,
                        if moved {
                            egui::pos2(365., 280.)
                        } else {
                            egui::pos2(400., 300.)
                        },
                        "{gesture:?}, consumed={consumed}"
                    );
                    if !middle {
                        assert!(owner_drag, "owner must still see its native gesture");
                        assert!(owner_release, "consumption must not erase owner release");
                    }
                }
            }
        }
    }
}
