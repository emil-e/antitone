use super::*;

#[test]
fn vectorscope_is_the_initial_editor_plot() {
    let scopes = Scopes::default();
    assert!(scopes.visible && scopes.vector);
    assert_eq!(scopes.amplification, 1);
}

#[test]
fn scope_feedback_reports_invalid_samples_outside_counts_and_errors() {
    use crate::scopes::Counts;
    let counts = Counts {
        valid: 100,
        invalid: 0,
        outside: 0,
    };
    assert_eq!(
        scope_feedback(Some(Ok(Counts {
            outside: 12,
            ..counts
        })))
        .unwrap()
        .0,
        "Outside plot: 12.0%"
    );
    let (text, color) = scope_feedback(Some(Ok(Counts {
        invalid: 3,
        outside: 12,
        ..counts
    })))
    .unwrap();
    assert_eq!(text, "Invalid rendered samples: 3");
    assert_eq!(color, egui::Color32::LIGHT_RED);
    assert_eq!(
        scope_feedback(Some(Err("Reduction failed".into()))).unwrap(),
        ("Reduction failed".into(), color)
    );
}

#[test]
fn scope_notices_stay_within_small_plots_and_bound_long_errors() {
    for dpi in [1., 2.] {
        let context = egui::Context::default();
        context.set_pixels_per_point(dpi);
        for width in [88., 250.] {
            let plot = egui::Rect::from_min_size(egui::pos2(10., 10.), egui::vec2(width, 96.));
            for align in [egui::Align2::LEFT_BOTTOM, egui::Align2::CENTER_CENTER] {
                for text in [
                    "No image".into(),
                    "Invalid rendered samples: 123".into(),
                    "GPU reduction failed: ".to_owned() + &"long diagnostic ".repeat(100),
                ] {
                    let long_error = text.len() > 100;
                    let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                        let truncated = scope_notice(
                            ui.painter(),
                            plot,
                            text.clone(),
                            egui::Color32::LIGHT_RED,
                            align,
                        );
                        assert_eq!(
                            truncated, long_error,
                            "only elided notices need hover details"
                        );
                    });
                    output.textures_delta.clear();
                    let text = output
                        .shapes
                        .iter()
                        .find_map(|shape| {
                            if let egui::Shape::Text(text) = &shape.shape {
                                assert_eq!(shape.clip_rect, plot);
                                Some(text)
                            } else {
                                None
                            }
                        })
                        .unwrap();
                    assert!(plot.contains_rect(text.visual_bounding_rect()));
                    assert!(text.galley.rows.len() <= 3);
                    assert_eq!(text.galley.elided, long_error);
                }
            }
        }
    }
}

#[test]
fn plot_switch_and_amplification_keep_slot_height_and_processing_state() {
    let context = egui::Context::default();
    for (width, height) in [(315., 600.), (270., 396.)] {
        let mut scopes = Scopes::default();
        let mut rectangles = Vec::new();
        for plot in [Plot::Rgb, Plot::Luma, Plot::Vector] {
            scopes.vector = plot == Plot::Vector;
            scopes.waveform = if plot == Plot::Luma {
                Plot::Luma
            } else {
                Plot::Rgb
            };
            scopes.amplification = 8;
            for _ in 0..3 {
                let mut slot = None;
                let mut output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, height),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        egui::CentralPanel::default().show(ui, |ui| {
                            slot = scopes.panel(ui, OutputTarget::DisplayP3, 120.);
                        });
                    },
                );
                output.textures_delta.clear();
                let slot = slot.unwrap();
                assert_eq!(slot.settings.plot, plot);
                assert_eq!(
                    slot.settings.amplification,
                    if plot == Plot::Vector { 8 } else { 1 }
                );
                assert!(slot.painter.clip_rect().contains_rect(slot.plot));
                rectangles.push(slot.plot);
            }
        }
        for pair in rectangles.windows(2) {
            assert_eq!(pair[0], pair[1]);
        }
    }
}

fn panel_frame(
    scopes: &mut Scopes,
    context: &egui::Context,
    size: egui::Vec2,
    events: Vec<egui::Event>,
) -> (Slot, egui::FullOutput) {
    let mut slot = None;
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                slot = scopes.panel(ui, OutputTarget::DisplayP3, 120.);
            });
        },
    );
    output.textures_delta.clear();
    (slot.unwrap(), output)
}

#[test]
fn scope_height_follows_width_and_is_bounded_in_short_windows() {
    let context = egui::Context::default();
    let mut scopes = Scopes::default();
    let mut previous = 0.;
    for width in [270., 315., 480.] {
        let (slot, output) = panel_frame(&mut scopes, &context, egui::vec2(width, 900.), vec![]);
        assert!(
            output.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::LineSegment { points, stroke }
                if points[0].y > slot.plot.bottom() && stroke.width > 0. && stroke.color.a() > 0)
            }),
            "divider must be visible even when idle"
        );
        assert!(
            (slot.plot.height() - slot.plot.width() * 0.85).abs() <= 0.5,
            "width={width}: {:?}",
            slot.plot
        );
        assert!(slot.plot.height() > previous);
        previous = slot.plot.height();
    }
    let (short, _) = panel_frame(&mut scopes, &context, egui::vec2(480., 360.), vec![]);
    assert!(short.plot.height() >= 96. && short.plot.height() < previous);
    assert!(short.painter.clip_rect().contains_rect(short.plot));
    assert!(short.plot.bottom() + 12. <= short.painter.clip_rect().bottom() - 120.);
    assert_eq!(
        scopes.height_ratio, 0.85,
        "window limits must not overwrite preference"
    );
}

#[test]
fn scope_divider_drag_release_reset_and_width_scaling_are_stable_at_both_dpis() {
    for dpi in [1., 2.] {
        let context = egui::Context::default();
        context.set_pixels_per_point(dpi);
        let mut scopes = Scopes::default();
        let size = egui::vec2(315., 900.);
        for _ in 0..3 {
            panel_frame(&mut scopes, &context, size, vec![]);
        }
        let (initial, _) = panel_frame(&mut scopes, &context, size, vec![]);
        let divider_center = |slot: &Slot| {
            slot.plot.center_bottom()
                + egui::vec2(0., context.global_style().spacing.item_spacing.y + 4.)
        };
        let start = divider_center(&initial);
        let end = start + egui::vec2(0., 60.);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let (_, hover) = panel_frame(
            &mut scopes,
            &context,
            size,
            vec![egui::Event::PointerMoved(start)],
        );
        assert_eq!(
            hover.platform_output.cursor_icon,
            egui::CursorIcon::ResizeVertical
        );
        panel_frame(&mut scopes, &context, size, vec![button(start, true)]);
        panel_frame(
            &mut scopes,
            &context,
            size,
            vec![egui::Event::PointerMoved(end)],
        );
        panel_frame(&mut scopes, &context, size, vec![button(end, false)]);
        let (resized, _) = panel_frame(&mut scopes, &context, size, vec![]);
        assert!(
            (resized.plot.height() - initial.plot.height() - 60.).abs() < 0.1,
            "dpi={dpi}: {:?} -> {:?}",
            initial.plot,
            resized.plot
        );
        for _ in 0..20 {
            let (idle, _) = panel_frame(&mut scopes, &context, size, vec![]);
            assert_eq!(idle.plot, resized.plot);
        }
        let ratio = scopes.height_ratio;
        let (wider, _) = panel_frame(&mut scopes, &context, egui::vec2(480., 900.), vec![]);
        assert!(
            (wider.plot.height() - wider.plot.width() * ratio).abs() <= 0.5 / dpi,
            "ratio={ratio}: {:?}",
            wider.plot
        );
        let (restored, _) = panel_frame(&mut scopes, &context, size, vec![]);
        let reset = divider_center(&restored);
        for _ in 0..2 {
            panel_frame(
                &mut scopes,
                &context,
                size,
                vec![egui::Event::PointerMoved(reset), button(reset, true)],
            );
            panel_frame(&mut scopes, &context, size, vec![button(reset, false)]);
        }
        let (reset, _) = panel_frame(&mut scopes, &context, size, vec![]);
        assert_eq!(scopes.height_ratio, 0.85);
        assert_eq!(reset.plot, initial.plot);
    }
}

#[test]
fn source_and_reference_views_suspend_scopes_without_dirtying_projects() {
    let (mut editor, _sender, _receiver) = super::super::tests::fixture();
    editor.sync_document();
    let project = editor.document.project.clone();
    for source in [true, false] {
        editor.source = source;
        editor.source_exposure = 2.;
        editor.scopes.vector = true;
        editor.scopes.amplification = 8;
        editor.scopes.height_ratio = 1.2;
        editor.scopes.visible = false;
        editor.sync_document();
        assert_eq!(editor.document.project, project);
        if source {
            assert_eq!(editor.scope_input().err(), Some("Positive grade only"));
        }
    }
    editor.busy = true;
    assert_eq!(editor.scope_input().err(), Some("Waiting for image"));
    editor.busy = false;
    assert_eq!(editor.scope_input().err(), Some("No image"));
}

#[test]
fn amplified_vector_graticule_is_clipped_to_the_square_trace_raster() {
    let context = egui::Context::default();
    let trace = egui::Rect::from_center_size(egui::pos2(150., 100.), egui::vec2(100., 100.));
    let settings = Settings {
        plot: Plot::Vector,
        amplification: 8,
        ..Default::default()
    };
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(300., 200.),
            )),
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| graticule(ui.painter(), trace, settings));
        },
    );
    output.textures_delta.clear();
    let circles: Vec<_> = output
        .shapes
        .iter()
        .filter(|shape| matches!(shape.shape, egui::Shape::Circle(_)))
        .collect();
    assert_eq!(circles.len(), 2);
    for circle in circles {
        assert_eq!(circle.clip_rect, trace);
    }
}

#[test]
fn vector_reference_markers_are_coloured_separately_from_the_graticule() {
    let context = egui::Context::default();
    let settings = Settings {
        plot: Plot::Vector,
        ..Default::default()
    };
    let mut output = context.run_ui(egui::RawInput::default(), |ui| {
        graticule(
            ui.painter(),
            egui::Rect::from_min_size(egui::pos2(10., 10.), egui::vec2(200., 200.)),
            settings,
        );
    });
    output.textures_delta.clear();
    let markers: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| {
            if let egui::Shape::Rect(rect) = &shape.shape {
                Some(rect.stroke.color)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(markers.len(), 6);
    assert_eq!(markers[0], egui::Color32::from_rgb(230, 80, 80));
    assert_eq!(markers[2], egui::Color32::from_rgb(80, 80, 230));
    for shape in &output.shapes {
        if let egui::Shape::Circle(circle) = &shape.shape {
            assert_eq!(circle.stroke.color, egui::Color32::from_rgb(82, 72, 110));
            assert!(!markers.contains(&circle.stroke.color));
        }
    }
}

#[test]
fn skin_guide_and_reference_boxes_follow_plot_geometry_at_every_gain() {
    let context = egui::Context::default();
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        for side in [88., 200.] {
            let trace = egui::Rect::from_min_size(egui::pos2(30., 30.), egui::Vec2::splat(side));
            let mut previous = None;
            for amplification in [1, 2, 4, 8] {
                let mut output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(300., 300.),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        graticule(
                            ui.painter(),
                            trace,
                            Settings {
                                target,
                                plot: Plot::Vector,
                                amplification,
                            },
                        )
                    },
                );
                output.textures_delta.clear();
                let mut boxes = Vec::new();
                let mut dashes = Vec::new();
                for shape in &output.shapes {
                    match &shape.shape {
                        egui::Shape::Rect(rect) => {
                            assert_eq!(shape.clip_rect, trace);
                            assert!(trace.contains_rect(rect.rect));
                            boxes.push(rect.rect);
                        }
                        egui::Shape::LineSegment { points, stroke }
                            if stroke.color == egui::Color32::from_rgb(160, 123, 68) =>
                        {
                            assert_eq!(shape.clip_rect, trace);
                            assert!(points.iter().all(|&point| trace.contains(point)));
                            assert!(points[0].distance(points[1]) <= 4.01);
                            dashes.push(*points);
                        }
                        _ => {}
                    }
                }
                let basis = Basis::new(target);
                let scale = side as f64 / (2. * basis.extent / amplification as f64);
                let expected: Vec<_> = references()
                    .into_iter()
                    .filter_map(|(_, rgb)| {
                        let point = basis.vector(rgb) * scale;
                        let marker = egui::Rect::from_center_size(
                            trace.center() + egui::vec2(point.x as f32, -point.y as f32),
                            egui::vec2(5., 5.),
                        );
                        trace.contains_rect(marker).then_some(marker)
                    })
                    .collect();
                assert_eq!(boxes, expected);
                if amplification == 1 {
                    assert_eq!(boxes.len(), 6);
                }
                if amplification == 8 {
                    assert!(boxes.is_empty());
                }
                assert!(dashes.len() >= 3);
                if let Some(old_dashes) = &previous {
                    assert_eq!(&dashes, old_dashes);
                }
                previous = Some(dashes);
            }
        }
    }
}

#[test]
fn waveform_has_no_skin_or_colour_direction_overlay() {
    let context = egui::Context::default();
    for plot in [Plot::Rgb, Plot::Luma] {
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            graticule(
                ui.painter(),
                egui::Rect::from_min_size(egui::pos2(30., 30.), egui::vec2(200., 200.)),
                Settings {
                    plot,
                    ..Default::default()
                },
            );
        });
        output.textures_delta.clear();
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape {
                    Some(text.galley.job.text.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(labels, ["0", "25", "50", "75", "100"]);
    }
}

#[test]
fn proxy_scopes_follow_visible_image_without_resizing_or_reprocessing_the_viewer() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let matrix = glam::DMat3::from_cols_array(&[0.9, 0.02, 0., 0.05, 1.1, 0., 0., 0.03, 0.8]);
        let shared_renderer = crate::renderer::GpuRenderer::new(&device, &queue)
            .await
            .unwrap();
        let area = Rect {
            x: 12,
            y: 34,
            width: 7000,
            height: 3000,
        };
        let path = PathBuf::from("scope-fixture.ARW");
        let ready = Ready {
            file: thumbnails::FileKey::new(&path),
            thumbnail: None,
            path: path.clone(),
            area,
            analysis_support: None,
            floor: crate::processing::CAMERA_INPUT_FLOOR,
            proxy: crate::viewer::GpuProxy::new(
                shared_renderer,
                &device,
                [7, 3],
                &[[4000, 3000, 2000, 0]; 21],
                matrix,
                None,
                crate::gpu::SourceGeometry::new([12, 34, 7000, 3000], [7, 3]).unwrap(),
            )
            .unwrap(),
            record: Source {
                id: SourceId(0),
                path,
                development: crate::project::SourceDevelopment {
                    lens_correction: None,
                    dimensions: [7040, 3072],
                    active_area: [12, 34, 7000, 3000],
                    camera_model: "test-camera".into(),
                    camera_to_ap1: matrix.to_cols_array_2d(),
                    input_floor: crate::processing::CAMERA_INPUT_FLOOR,
                },
                reference_only: false,
                photo: Default::default(),
                local_adjustments: None,
                repairs: Default::default(),
                orientation: Default::default(),
                crop: None,
            },
            development: Development::new(crate::raw::version(), [2., 1., 1.5, 1.]),
        };
        let (mut editor, _send, _receiver) = super::super::tests::fixture();
        editor.ready = Some(ready);
        editor
            .document
            .project
            .camera_basis
            .as_mut()
            .unwrap()
            .camera_to_ap1 = matrix.to_cols_array_2d();
        editor.source = false;
        editor.scopes = Scopes::new(&device);
        let gpu = editor.scopes.gpu.as_ref().unwrap().clone();
        let context = egui::Context::default();
        let mut renderer = egui_wgpu::Renderer::new(
            &device,
            wgpu::TextureFormat::Rgba16Float,
            Default::default(),
        );
        let mut prepare = |output: egui::FullOutput, size: egui::Vec2| {
            let screen = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [(size.x * 2.) as u32, (size.y * 2.) as u32],
                pixels_per_point: 2.,
            };
            let jobs = context.tessellate(output.shapes, output.pixels_per_point);
            let mut encoder = device.create_command_encoder(&Default::default());
            let commands = renderer.update_buffers(&device, &queue, &mut encoder, &jobs, &screen);
            queue.submit(commands.into_iter().chain([encoder.finish()]));
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        };
        let frame = |editor: &mut Editor, size: egui::Vec2| {
            context.set_pixels_per_point(2.);
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ui| host::App::ui(editor, ui),
            );
            output.textures_delta.clear();
            output
        };
        for size in [egui::vec2(1100., 800.), egui::vec2(800., 600.)] {
            for _ in 0..3 {
                frame(&mut editor, size);
            }
            editor.scopes.visible = false;
            let without = frame(&mut editor, size);
            let viewer_clip = without
                .shapes
                .iter()
                .find_map(|shape| {
                    if let egui::Shape::Callback(callback) = &shape.shape {
                        assert!(callback.callback.is::<egui_wgpu::Callback>());
                        Some(shape.clip_rect)
                    } else {
                        None
                    }
                })
                .unwrap();
            editor.scopes.visible = true;
            for _ in 0..3 {
                frame(&mut editor, size);
            }
            let with = frame(&mut editor, size);
            let mut found = false;
            for shape in &with.shapes {
                if let egui::Shape::Callback(callback) = &shape.shape {
                    assert!(shape.clip_rect.expand(0.1).contains_rect(callback.rect));
                    if callback.rect.center().x > viewer_clip.right() {
                        found = true;
                    } else {
                        assert_eq!(shape.clip_rect, viewer_clip);
                    }
                }
            }
            assert!(found);
            let (input, _, fit_frame) = editor.scope_input().unwrap();
            assert!(Arc::ptr_eq(
                &input,
                &editor
                    .ready
                    .as_ref()
                    .unwrap()
                    .proxy
                    .image(editor.target)
                    .unwrap()
            ));
            prepare(with, size);
            let reductions = gpu.reduction_count();
            editor.viewport.zoom = Some(2.);
            prepare(frame(&mut editor, size), size);
            let zoom_frame = editor.scope_input().unwrap().2;
            assert_ne!(zoom_frame, fit_frame);
            assert!(zoom_frame.min.0[0] > fit_frame.min.0[0]);
            assert!(zoom_frame.max.0[0] < fit_frame.max.0[0]);
            assert_eq!(zoom_frame.output, [1, 1]);
            assert_eq!(gpu.reduction_count(), reductions + 1);
            let reductions = gpu.reduction_count();
            prepare(frame(&mut editor, size), size);
            assert_eq!(gpu.reduction_count(), reductions, "idle view reuses bins");
            let (layout, ppp) = editor.last_layout.unwrap();
            editor.viewport.pan(layout, ppp, egui::vec2(100., -60.));
            prepare(frame(&mut editor, size), size);
            let pan_frame = editor.scope_input().unwrap().2;
            assert_ne!(pan_frame, zoom_frame);
            assert_eq!(pan_frame.output, zoom_frame.output);
            assert_eq!(gpu.reduction_count(), reductions + 1);
            editor.viewport.fit();
        }
        prepare(
            frame(&mut editor, egui::vec2(1100., 800.)),
            egui::vec2(1100., 800.),
        );
        let (image, input, scope_frame) = editor.scope_input().unwrap();
        let parameters = input.parameters();
        let expected = ProcessingParameters::positive(
            editor.calibration().unwrap(),
            editor.roll,
            editor.photo,
            matrix,
        )
        .unwrap();
        assert_eq!(
            bytemuck::bytes_of(&parameters),
            bytemuck::bytes_of(&expected)
        );
        let settings = Settings {
            target: editor.target,
            plot: Plot::Vector,
            ..Default::default()
        };
        assert_eq!(
            gpu.status(&image, parameters, scope_frame, settings)
                .unwrap()
                .unwrap()
                .valid,
            21
        );
        editor.roll_orientation.turn_clockwise();
        editor.photo_orientation.flips.horizontal = true;
        editor.crop = Some(
            Crop {
                center: [3512., 1534.],
                size: [5000, 1000],
                angle: 13_f64.to_radians(),
            }
            .constrained(area)
            .unwrap(),
        );
        let (_, composed_input, composed_frame) = editor.scope_input().unwrap();
        let composed_parameters = composed_input.parameters();
        assert_eq!(
            bytemuck::bytes_of(&parameters),
            bytemuck::bytes_of(&composed_parameters)
        );
        assert_eq!(composed_frame.output, [1, 5]);
        assert_eq!(
            composed_frame.composition,
            Composition::with_crop(
                area,
                editor.roll_orientation,
                editor.photo_orientation,
                editor.crop
            )
            .unwrap()
        );
        assert!(
            gpu.status(&image, parameters, composed_frame, settings)
                .is_none()
        );
        prepare(
            frame(&mut editor, egui::vec2(1100., 800.)),
            egui::vec2(1100., 800.),
        );
        assert_eq!(
            gpu.status(&image, parameters, composed_frame, settings)
                .unwrap()
                .unwrap()
                .valid,
            5
        );
        assert!(
            gpu.status(&image, parameters, scope_frame, settings)
                .is_none()
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        assert!(
            !image.prepare(&device, &mut encoder, parameters),
            "geometry must not rerender ACES"
        );
        editor.viewport.zoom = Some(3.);
        let zoomed_composed = editor.scope_input().unwrap().2;
        assert_ne!(zoomed_composed, composed_frame);
        assert_eq!(zoomed_composed.composition, composed_frame.composition);
        assert_eq!(zoomed_composed.output, [1, 1]);
        editor.begin_crop();
        assert_eq!(
            editor.scope_input().err(),
            Some("Apply crop to update scopes")
        );
        frame(&mut editor, egui::vec2(1100., 800.));
        assert!(
            gpu.status(&image, parameters, composed_frame, settings)
                .is_none()
        );
        editor.finish_crop(false);
        assert_eq!(editor.scope_input().err(), Some("Waiting for view"));
        prepare(
            frame(&mut editor, egui::vec2(1100., 800.)),
            egui::vec2(1100., 800.),
        );
        let before = editor.document.project.clone();
        editor.source = true;
        frame(&mut editor, egui::vec2(800., 600.));
        assert!(
            gpu.status(&image, parameters, composed_frame, settings)
                .is_none()
        );
        assert_eq!(editor.document.project, before);
    });
}
