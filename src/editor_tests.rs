use super::*;
fn fixture() -> (Editor, mpsc::Sender<Message>, mpsc::Receiver<Command>) {
    let (commands, receiver) = mpsc::channel();
    let (send, messages) = mpsc::channel();
    let mut editor = Editor::state(
        commands,
        messages,
        Arc::new(std::sync::atomic::AtomicU64::new(0)),
    );
    editor.dmin = Some(DVec3::new(0.2, 0.15, 0.1));
    editor.dmax = Some(DVec3::new(0.008, 0.005, 0.002));
    editor.floor = 2.2e-5;
    editor.document.project.development =
        Some(Development::new(crate::raw::version(), [2., 1., 1.5, 1.]));
    (editor, send, receiver)
}
#[test]
fn fit_reference_and_picker_actions_stay_inside_the_visible_viewer() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let images = [
            Arc::new(
                GpuImage::new_rendered(&device, &queue, 16, 16, OutputTarget::Srgb)
                    .await
                    .unwrap(),
            ),
            Arc::new(
                GpuImage::new_rendered(&device, &queue, 16, 16, OutputTarget::DisplayP3)
                    .await
                    .unwrap(),
            ),
        ];
        let area = Rect {
            x: 12,
            y: 12,
            width: 8640,
            height: 5760,
        };
        for (width, height, ppp) in [(1100., 800., 1.), (800., 600., 2.)] {
            for kind in [Pick::Flat, Pick::Dmin] {
                let (mut editor, _send, _receiver) = fixture();
                editor.dmin = None;
                editor.dmax = None;
                editor.source = true;
                editor.pick = Some(kind);
                editor.region = Some(Rect {
                    x: 46,
                    y: 46,
                    width: 8602,
                    height: 5508,
                });
                let path = PathBuf::from(format!(
                    "/Users/example/{}/reference-with-a-long-filename.ARW",
                    "a-long-scanning-session-directory/".repeat(12)
                ));
                editor.ready = Some(Ready {
                    path: path.clone(),
                    area,
                    floor: editor.floor,
                    images: images.clone(),
                    record: Source {
                        id: SourceId(0),
                        path,
                        fingerprint: crate::project::Fingerprint {
                            bytes: 1,
                            blake3: "0".repeat(64),
                        },
                        development: crate::project::SourceDevelopment {
                            dimensions: [8704, 6144],
                            active_area: [12, 12, 8640, 5760],
                            camera_to_ap0: glam::DMat3::IDENTITY.to_cols_array_2d(),
                            input_floor: editor.floor,
                        },
                        reference_only: true,
                        photo: Default::default(),
                    },
                    development: editor.document.project.development.clone().unwrap(),
                });
                let context = egui::Context::default();
                context.set_pixels_per_point(ppp);
                let mut fit_point = None;
                for frame in 0..5 {
                    let events = if frame < 3 {
                        vec![]
                    } else {
                        let pos = fit_point.unwrap();
                        if frame == 4 {
                            editor.viewport.zoom = Some(2.);
                        }
                        vec![
                            egui::Event::PointerMoved(pos),
                            egui::Event::PointerButton {
                                pos,
                                button: egui::PointerButton::Primary,
                                pressed: frame == 3,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ]
                    };
                    let mut output = context.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(width, height),
                            )),
                            events,
                            ..Default::default()
                        },
                        |ui| host::App::ui(&mut editor, ui),
                    );
                    output.textures_delta.clear();
                    let mut image = false;
                    let mut rectangle = false;
                    let mut cancel = false;
                    for shape in output.shapes {
                        match shape.shape {
                            egui::Shape::Callback(callback) => {
                                image = true;
                                assert!(
                                    shape.clip_rect.expand(0.1).contains_rect(callback.rect),
                                    "Fit image exceeds visible viewer: {:?} / {:?}",
                                    callback.rect,
                                    shape.clip_rect
                                );
                                assert!(
                                    (callback.rect.aspect_ratio()
                                        - area.width as f32 / area.height as f32)
                                        .abs()
                                        < 1e-4,
                                    "Fit did not restore aspect: frame={frame} kind={kind:?} zoom={:?} image={:?}",
                                    editor.viewport.zoom,
                                    callback.rect
                                );
                            }
                            egui::Shape::Rect(rect)
                                if rect.stroke.color == egui::Color32::WHITE
                                    && rect.stroke.width == 1. =>
                            {
                                rectangle = true;
                                assert!(
                                    shape.clip_rect.expand(0.1).contains_rect(rect.rect),
                                    "proposed support exceeds visible viewer"
                                );
                            }
                            egui::Shape::Text(text) if text.galley.job.text == "Fit" => {
                                fit_point = Some(
                                    egui::Rect::from_min_size(text.pos, text.galley.size())
                                        .center(),
                                );
                            }
                            egui::Shape::Text(text) if text.galley.job.text == "Cancel pick" => {
                                cancel = true;
                                let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
                                assert!(
                                    shape.clip_rect.expand(0.1).contains_rect(rect),
                                    "Cancel is outside the visible picker header"
                                );
                            }
                            _ => {}
                        }
                    }
                    assert!(image && rectangle && cancel);
                    assert!(editor.viewport.zoom.is_none());
                }
            }
        }
    });
}
#[test]
fn source_inspection_is_transient_and_pick_commands_never_receive_its_gain() {
    let (mut editor, send, receiver) = fixture();
    editor.sync_document();
    let project = editor.document.project.clone();
    let settings = (editor.dmin, editor.dmax, editor.roll, editor.photo);
    editor.source_exposure = 2.3456789;
    editor.source = true;
    editor.sync_document();
    assert_eq!(editor.document.project, project);
    editor.begin_pick(Pick::Dmin);
    editor.region = Some(patch());
    editor.apply_pick();
    let Command::Pick(id, kind, _, _, roll, photo) = receiver.try_recv().unwrap() else {
        panic!()
    };
    assert_eq!(roll, settings.2);
    assert_eq!(photo, settings.3);
    send.send(Message::Picked(
        id,
        kind,
        Ok(Measurement::Reference(settings.0.unwrap())),
    ))
    .unwrap();
    editor.receive();
    assert_eq!(
        (editor.dmin, editor.dmax, editor.roll, editor.photo),
        settings
    );
    assert_eq!(editor.source_exposure, 2.3456789);
    editor.sync_document();
    assert_eq!(editor.document.project, project);
}
#[test]
fn rejected_reference_and_cancel_restore_view_without_changing_processing() {
    let (mut editor, send, _receiver) = fixture();
    editor.source = false;
    let original = (editor.dmin, editor.dmax, editor.roll, editor.photo);
    for (kind, invalid) in [(Pick::Dmin, 0.001), (Pick::Dmax, 1.)] {
        editor.begin_pick(kind);
        assert!(editor.source);
        editor.region = Some(patch());
        editor.apply_pick();
        send.send(Message::Picked(
            editor.id,
            kind,
            Ok(Measurement::Reference(DVec3::splat(invalid))),
        ))
        .unwrap();
        editor.receive();
        assert!(!editor.source);
        assert!(!editor.busy);
        assert!(editor.pick.is_none());
        assert!(editor.region.is_none());
        assert_eq!(
            (editor.dmin, editor.dmax, editor.roll, editor.photo),
            original
        );
        editor.begin_pick(kind);
        assert!(editor.source);
        editor.finish_pick();
        assert!(!editor.source);
        assert_eq!(
            (editor.dmin, editor.dmax, editor.roll, editor.photo),
            original
        );
    }
    editor.source = true;
    editor.begin_pick(Pick::Dmax);
    editor.finish_pick();
    assert!(editor.source);
}
#[test]
fn neutral_proposals_have_independent_scopes_and_reject_invalid_gpu_settings() {
    let (mut editor, send, _receiver) = fixture();
    editor.source = false;
    editor.photo.exposure = 0.5;
    editor.photo.wb = Balance {
        warmth: 0.2,
        tint: 0.1,
    };
    editor.roll.exposure = -0.5;
    let photo = editor.photo;
    let gamma = editor.roll.gamma;
    let exposure = editor.roll.exposure;
    let balance = Balance {
        warmth: 0.1,
        tint: -0.1,
    };
    editor.begin_pick(Pick::Gamma);
    send.send(Message::Picked(
        editor.id,
        Pick::Gamma,
        Ok(Measurement::Balance(balance)),
    ))
    .unwrap();
    editor.receive();
    assert_eq!(editor.roll.balance, balance);
    assert_eq!(editor.roll.gamma, gamma);
    assert_eq!(editor.roll.exposure, exposure);
    assert_eq!(editor.photo, photo);
    let roll = editor.roll;
    editor.begin_pick(Pick::Wb);
    send.send(Message::Picked(
        editor.id,
        Pick::Wb,
        Ok(Measurement::Balance(balance)),
    ))
    .unwrap();
    editor.receive();
    assert_eq!(editor.roll, roll);
    assert_eq!(editor.photo.exposure, photo.exposure);
    assert_eq!(editor.photo.wb, balance);
    editor.roll.gamma = 0.1;
    let old = editor.roll.balance;
    editor.begin_pick(Pick::Gamma);
    send.send(Message::Picked(
        editor.id,
        Pick::Gamma,
        Ok(Measurement::Balance(Balance {
            warmth: 1.,
            tint: 1.,
        })),
    ))
    .unwrap();
    editor.receive();
    assert_eq!(editor.roll.balance, old);
    assert!(editor.status.contains("rejected"));
    assert!(!editor.source);
}
#[test]
fn stopped_pick_worker_clears_busy_and_stale_samples_do_not_apply() {
    let (mut editor, send, receiver) = fixture();
    editor.source = false;
    drop(receiver);
    editor.begin_pick(Pick::Dmin);
    editor.region = Some(patch());
    editor.apply_pick();
    assert!(!editor.busy);
    assert!(editor.status.contains("worker stopped"));
    assert!(!editor.source);
    let old = editor.photo;
    send.send(Message::Picked(
        editor.id + 1,
        Pick::Wb,
        Ok(Measurement::Balance(Balance {
            warmth: 0.4,
            tint: 0.4,
        })),
    ))
    .unwrap();
    editor.receive();
    assert_eq!(editor.photo, old);
}
#[test]
fn navigation_invalidates_pending_overwrite() {
    let (mut editor, _send, receiver) = fixture();
    editor.overwrite = Some((
        "existing.tif".into(),
        OutputFormat::Tiff(OutputTarget::DisplayP3),
    ));
    editor.load("another.arw".into());
    assert!(editor.overwrite.is_none());
    assert!(matches!(
        receiver.recv().unwrap(),
        Command::Load(1, _, _, _, _)
    ));
}
fn patch() -> Rect {
    Rect {
        x: 12,
        y: 12,
        width: 32,
        height: 32,
    }
}
#[test]
fn repicking_uses_current_selection_and_starts_without_a_rectangle() {
    let (mut editor, send, receiver) = fixture();
    for path in ["previous.arw", "current.arw"] {
        editor
            .document
            .register(
                Source {
                    id: SourceId(0),
                    path: path.into(),
                    fingerprint: crate::project::Fingerprint {
                        bytes: 1,
                        blake3: "a".repeat(64),
                    },
                    development: crate::project::SourceDevelopment {
                        dimensions: [900, 600],
                        active_area: [0, 0, 900, 600],
                        camera_to_ap0: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                        input_floor: 2.2e-5,
                    },
                    reference_only: false,
                    photo: PhotoSettings::default(),
                },
                Development::new(crate::raw::version(), [2., 1., 1.5, 1.]),
            )
            .unwrap();
    }
    editor.selected = 1;
    editor.region = Some(patch());
    editor.begin_pick(Pick::Dmin);
    assert_eq!(editor.selected, 1);
    assert!(editor.region.is_none());
    assert!(receiver.try_recv().is_err());
    let value = DVec3::new(0.3, 0.2, 0.15);
    send.send(Message::Picked(
        editor.id,
        Pick::Dmin,
        Ok(Measurement::Reference(value)),
    ))
    .unwrap();
    editor.receive();
    assert_eq!(editor.dmin, Some(value));
    editor.document.project.order.clear();
    assert!(editor.calibration().is_some());
    editor.begin_pick(Pick::Dmax);
    assert!(editor.region.is_none());
    assert!(receiver.try_recv().is_err());
}
#[test]
fn rectangle_mapping_is_independent_of_view_scale() {
    let area = Rect {
        x: 12,
        y: 12,
        width: 8640,
        height: 5760,
    };
    let rectangle = |canvas, area, start, end| {
        Viewport::default()
            .layout(canvas, area, 1.)
            .rectangle(start, end)
    };
    let a = rectangle(
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900., 600.)),
        area,
        egui::pos2(100., 200.),
        egui::pos2(300., 400.),
    );
    let b = rectangle(
        egui::Rect::from_min_size(egui::pos2(50., 20.), egui::vec2(450., 300.)),
        area,
        egui::pos2(100., 120.),
        egui::pos2(200., 220.),
    );
    assert_eq!((a.x, a.y, a.width, a.height), (b.x, b.y, b.width, b.height));
}

fn controls_frame(
    editor: &mut Editor,
    context: &egui::Context,
    width: f32,
    events: Vec<egui::Event>,
) -> Vec<(String, egui::Rect)> {
    controls_frame_sized(editor, context, width, 1300., events)
}
fn controls_frame_sized(
    editor: &mut Editor,
    context: &egui::Context,
    width: f32,
    height: f32,
    events: Vec<egui::Event>,
) -> Vec<(String, egui::Rect)> {
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, height),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| editor.controls(ui));
        },
    );
    output.textures_delta.clear();
    output
        .shapes
        .into_iter()
        .filter_map(|shape| {
            if let egui::Shape::Text(text) = shape.shape {
                Some((
                    text.galley.job.text.clone(),
                    egui::Rect::from_min_size(text.pos, text.galley.size()),
                ))
            } else {
                None
            }
        })
        .collect()
}
fn click_section_reset(editor: &mut Editor, context: &egui::Context, name: &str) {
    let text = controls_frame(editor, context, 315., vec![]);
    let title = text.iter().find(|(label, _)| label == name).unwrap().1;
    let reset = text
        .iter()
        .find(|(label, rect)| {
            label == "↺"
                && (rect.center().y - title.center().y).abs() < 3.
                && rect.left() > title.right()
        })
        .unwrap()
        .1
        .center();
    for pressed in [true, false] {
        controls_frame(
            editor,
            context,
            315.,
            vec![
                egui::Event::PointerMoved(reset),
                egui::Event::PointerButton {
                    pos: reset,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
}
#[test]
fn structured_controls_fit_normal_and_narrow_widths_without_an_empty_stage() {
    for width in [270., 315.] {
        let context = egui::Context::default();
        let (mut editor, _send, _receiver) = fixture();
        for tab in [0, 1] {
            editor.tab = tab;
            let text = controls_frame(&mut editor, &context, width, vec![]);
            let labels: Vec<_> = text.iter().map(|(label, _)| label.as_str()).collect();
            let required = if tab == 0 {
                vec![
                    "Calibration",
                    "Gamma & balance",
                    "Gamma balance",
                    "Toe & shoulder",
                    "Toe",
                    "Shoulder",
                    "Exposure",
                    "Output",
                ]
            } else {
                vec!["White balance", "Exposure", "Output"]
            };
            for label in &required {
                assert!(labels.contains(label), "missing {label}");
            }
            let sections = if tab == 0 {
                vec![
                    "Calibration",
                    "Gamma & balance",
                    "Toe & shoulder",
                    "Exposure",
                ]
            } else {
                vec!["White balance", "Exposure"]
            };
            for adjacent in sections.windows(2) {
                let a = text
                    .iter()
                    .find(|(label, _)| label == adjacent[0])
                    .unwrap()
                    .1;
                let b = text
                    .iter()
                    .find(|(label, _)| label == adjacent[1])
                    .unwrap()
                    .1;
                assert!(a.top() < b.top(), "wrong section order: {adjacent:?}");
            }
            for (label, rect) in text {
                assert!(
                    rect.left() >= 0. && rect.right() <= width + 1.,
                    "{label}: {rect:?} / {width}"
                );
                assert!(
                    !["Roll post", "Neutral", "SHARED • fixed development WB"]
                        .contains(&label.as_str())
                );
            }
        }
    }
}
#[test]
fn section_and_color_resets_preserve_calibration_and_other_edit_scopes() {
    let context = egui::Context::default();
    let (mut editor, _send, _receiver) = fixture();
    editor.roll.gamma = 1.1;
    editor.roll.balance = Balance {
        warmth: 0.3,
        tint: 0.2,
    };
    editor.roll.exposure = 2.;
    editor.photo = PhotoSettings {
        exposure: -1.,
        wb: Balance {
            warmth: -0.2,
            tint: 0.1,
        },
    };
    let calibration = (editor.dmin, editor.dmax);
    let photo = editor.photo;
    let shape = crate::processing::Response::Film {
        toe: 3.,
        shoulder: 0.01,
    };
    editor.roll.response = shape;
    click_section_reset(&mut editor, &context, "Gamma & balance");
    assert_eq!(editor.roll.response, shape);
    assert_eq!(editor.roll.gamma, crate::processing::DEFAULT_GAMMA);
    assert_eq!(editor.roll.balance, Balance::default());
    assert_eq!(editor.roll.exposure, 2.);
    assert_eq!(editor.photo, photo);
    editor.roll.gamma = 0.9;
    editor.roll.balance = Balance {
        warmth: 0.2,
        tint: -0.1,
    };
    click_section_reset(&mut editor, &context, "Gamma balance");
    assert_eq!(editor.roll.gamma, 0.9);
    assert_eq!(editor.roll.balance, Balance::default());
    click_section_reset(&mut editor, &context, "Toe & shoulder");
    assert_eq!(editor.roll.response, crate::processing::Response::film());
    assert_eq!(editor.roll.gamma, 0.9);
    assert_eq!(editor.roll.balance, Balance::default());
    assert_eq!(editor.roll.exposure, 2.);
    assert_eq!(editor.photo, photo);
    click_section_reset(&mut editor, &context, "Exposure");
    assert_eq!(editor.roll.exposure, 0.);
    assert_eq!(editor.roll.gamma, 0.9);
    assert_eq!(editor.photo, photo);
    let roll = editor.roll;
    editor.tab = 1;
    click_section_reset(&mut editor, &context, "White balance");
    assert_eq!(editor.photo.wb, Balance::default());
    assert_eq!(editor.photo.exposure, -1.);
    click_section_reset(&mut editor, &context, "Exposure");
    assert_eq!(editor.photo.exposure, 0.);
    assert_eq!(editor.roll, roll);
    assert_eq!((editor.dmin, editor.dmax), calibration);
}

#[test]
fn response_section_is_reachable_by_scrolling_and_output_stays_pinned() {
    for (width, height) in [(315., 800.), (270., 600.)] {
        let context = egui::Context::default();
        let (mut editor, _send, _receiver) = fixture();
        for _ in 0..3 {
            controls_frame_sized(&mut editor, &context, width, height, vec![]);
        }
        let first = controls_frame_sized(&mut editor, &context, width, height, vec![]);
        let output = first.iter().find(|(label, _)| label == "Output").unwrap().1;
        for frame in 0..60 {
            let events = if frame == 0 {
                vec![
                    egui::Event::PointerMoved(egui::pos2(width / 2., 300.)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        phase: egui::TouchPhase::Move,
                        delta: egui::vec2(0., -1000.),
                        modifiers: egui::Modifiers::NONE,
                    },
                ]
            } else {
                vec![]
            };
            controls_frame_sized(&mut editor, &context, width, height, events);
        }
        let scrolled = controls_frame_sized(&mut editor, &context, width, height, vec![]);
        for name in ["Toe & shoulder", "Toe", "Shoulder", "Exposure"] {
            let rect = scrolled.iter().find(|(label, _)| label == name).unwrap().1;
            assert!(
                rect.top() > 0. && rect.bottom() < output.top(),
                "{name}: {rect:?}"
            );
        }
        assert_eq!(
            scrolled
                .iter()
                .find(|(label, _)| label == "Output")
                .unwrap()
                .1,
            output
        );
    }
}

#[test]
fn original_grade_requires_explicit_response_opt_in_and_drawing_never_regrades_it() {
    let context = egui::Context::default();
    let (mut editor, _send, _receiver) = fixture();
    editor.roll.response = crate::processing::Response::Original;
    editor.roll.gamma = 0.73;
    editor.roll.exposure = 2.345;
    let before = editor.roll;
    let references = (editor.dmin, editor.dmax);
    let photo = editor.photo;
    let text = controls_frame(&mut editor, &context, 315., vec![]);
    assert_eq!(editor.roll, before);
    assert!(
        !text
            .iter()
            .any(|(label, _)| label == "Toe" || label == "Shoulder")
    );
    let button = text
        .iter()
        .find(|(label, _)| label == "Use adjustable response")
        .unwrap()
        .1
        .center();
    for pressed in [true, false] {
        controls_frame(
            &mut editor,
            &context,
            315.,
            vec![
                egui::Event::PointerMoved(button),
                egui::Event::PointerButton {
                    pos: button,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    assert_eq!(
        editor.roll,
        RollSettings {
            response: crate::processing::Response::film(),
            ..before
        }
    );
    assert_eq!((editor.dmin, editor.dmax), references);
    assert_eq!(editor.photo, photo);
}
