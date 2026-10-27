use super::*;
use crate::looks::builtin::{Builtin, DEFAULT_GRID};
fn tiff_codes(path: &std::path::Path) -> Vec<u16> {
    let mut decoder = tiff::decoder::Decoder::new(std::fs::File::open(path).unwrap()).unwrap();
    let tiff::decoder::DecodingResult::U16(codes) = decoder.read_image().unwrap() else {
        panic!("not RGB16")
    };
    codes
}

fn paint_until(harness: &mut Harness<'_, Editor>, done: impl Fn(&Editor) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        harness.step();
        harness.render().unwrap();
        if harness.read(&done) {
            break;
        }
        harness.read(|editor| {
            assert!(
                Instant::now() < deadline,
                "look paint timed out: {} {:?}",
                editor.status,
                editor.working.error
            )
        });
        std::thread::sleep(Duration::from_millis(5));
    }
    harness.step();
}
fn positive_ready(editor: &Editor) -> bool {
    !editor.busy
        && editor.working.displayed.as_ref().is_some_and(|view| {
            let mut expected = editor
                .positive_processing(editor.input, editor.roll, editor.adjustment_layer_settings)
                .unwrap();
            expected.repairs = crate::gpu::repairs::Snapshot::logical(
                editor
                    .dust_signature()
                    .map(|signature| editor.dust_evaluations(signature))
                    .unwrap_or_default(),
            )
            .unwrap();
            view.processing == expected.signature()
                && view.target == editor.target
                && !view.in_flight()
                && matches!(view.image.look_result(), Some(Ok(())))
        })
}
fn fixture_editor<R>(test: impl FnOnce(Harness<'_, Editor>, Rc<RefCell<FloatCapture>>) -> R) -> R {
    fixture_editor_gpu(|harness, capture, _, _| test(harness, capture))
}

fn fixture_editor_gpu<R>(
    test: impl FnOnce(Harness<'_, Editor>, Rc<RefCell<FloatCapture>>, wgpu::Device, wgpu::Queue) -> R,
) -> R {
    editor_harness_with_gpu(true, |mut harness, capture, device, queue| {
        harness.run();
        harness.enter(|editor, cx| {
            editor.add_raws(vec![fixture("roll-0.dng"), fixture("roll-1.dng")], cx)
        });
        wait_for(&mut harness, |editor| {
            !editor.busy && editor.document.visit.ready.is_some()
        });
        harness.update(|editor| {
            editor.set_dmin(Some(DVec3::splat(3000. / 4031.)));
            editor.set_dmax(Some(DVec3::splat(128. / 4031.)));
            editor.source = false;
            editor.scopes.visible = true;
            editor.sync_document();
        });
        harness.step();
        stage(&mut harness, "Rendering");
        paint_until(&mut harness, positive_ready);
        test(harness, capture, device, queue)
    })
}

fn choose(harness: &mut Harness<'_, Editor>, name: &str) {
    harness
        .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Look")
        .scroll_to_me();
    harness.try_run_realtime().unwrap();
    harness
        .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Look")
        .click();
    harness.step();
    harness.step();
    harness.get_by_label(name).scroll_to_me();
    harness.try_run_realtime().unwrap();
    harness.get_by_label(name).click();
    harness.step();
}
#[test]
fn rendering_dropdowns_share_section_geometry_and_target_help() {
    editor_harness(|mut harness, _| {
        harness.update(|editor| editor.scopes.visible = false);
        harness.run();
        for scale in [1., 2.] {
            harness
                .input_mut()
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .native_pixels_per_point = Some(scale);
            for (size, width, name) in [
                (egui::vec2(1100., 800.), 315., "normal"),
                (egui::vec2(800., 600.), 270., "narrow"),
            ] {
                set_layout(&mut harness, size, width);
                stage(&mut harness, "Rendering");
                harness.event(egui::Event::PointerGone);
                harness.run();
                let look = harness
                    .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Look")
                    .rect();
                let target = harness
                    .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Target")
                    .rect();
                assert_eq!(look.x_range(), target.x_range());
                assert_eq!(look.height(), presentation::CONTROL_HEIGHT);
                assert_eq!(target.height(), look.height());
                assert!(look.left() >= size.x - width && look.right() <= size.x);
                for (label, rect) in [("Look", look), ("Target", target)] {
                    let headings: Vec<_> = harness
                        .output()
                        .shapes
                        .iter()
                        .filter_map(|shape| match &shape.shape {
                            egui::Shape::Text(text) if text.galley.job.text == label => Some(text),
                            _ => None,
                        })
                        .collect();
                    assert_eq!(headings.len(), 1, "section heading supplies the label");
                    assert!(headings[0].pos.y < rect.top());
                }
                if let Some(output) = std::env::var_os("ANTITONE_UI_CAPTURE") {
                    let output = PathBuf::from(output);
                    std::fs::create_dir_all(&output).unwrap();
                    harness
                        .render()
                        .unwrap()
                        .save(output.join(format!("rendering-dropdowns-{name}-{scale}x.png")))
                        .unwrap();
                }
                // Hover must follow the stage click, not fall inside egui's click suppression.
                let time = harness.ctx.input(|input| input.time);
                harness.input_mut().time = Some(time + 0.2);
                harness.step();
                harness
                    .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Target")
                    .hover();
                harness.step();
                let time = harness.ctx.input(|input| input.time);
                harness.input_mut().time = Some(time + 1.);
                harness.step();
                harness.step();
                assert!(
                    harness
                        .query_by_label("Output colour space, not the monitor profile.")
                        .is_some()
                );
                select_target(&mut harness, OutputTarget::Srgb);
                harness.get_by_label("Reset Target").click();
                harness.run();
                harness.read(|editor| assert_eq!(editor.target, OutputTarget::DisplayP3));
            }
        }
    });
}

#[test]
fn look_browsing_exposes_import_and_attribution_without_accepting_a_change() {
    fixture_editor(|mut harness, _| {
        let before = harness.read(|editor| editor.document.project.clone());
        for (size, name) in [
            (egui::vec2(1100., 800.), "normal"),
            (egui::vec2(800., 600.), "narrow"),
        ] {
            harness.set_size(size);
            harness.run();
            harness
                .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Look")
                .click();
            harness.step();
            harness.step();
            let load = harness.get_by_label("Load LUT…");
            assert!(!load.accesskit_node().is_disabled());
            for builtin in Builtin::ALL {
                assert!(harness.query_by_label(builtin.name()).is_some());
            }
            if let Some(output) = std::env::var_os("ANTITONE_UI_CAPTURE") {
                let output = PathBuf::from(output);
                std::fs::create_dir_all(&output).unwrap();
                harness
                    .render()
                    .unwrap()
                    .save(output.join(format!("look-menu-{name}-offscreen.png")))
                    .unwrap();
            }
            let time = harness.ctx.input(|input| input.time);
            harness.input_mut().time = Some(time + 0.2);
            harness.step();
            harness.get_by_label("Cinema Print").hover();
            harness.step();
            let time = harness.ctx.input(|input| input.time);
            harness.input_mut().time = Some(time + 1.);
            harness.step();
            harness.step();
            assert!(harness.query_by_label("Independent creative interpretation derived from published KODAK VISION Color Print Film 2383 / 3383 data.\nNot affiliated with or endorsed by the manufacturer.").is_some());
            if let Some(output) = std::env::var_os("ANTITONE_UI_CAPTURE") {
                harness
                    .render()
                    .unwrap()
                    .save(PathBuf::from(output).join(format!("look-source-{name}-offscreen.png")))
                    .unwrap();
            }
            harness.event(egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
            harness.run();
            assert!(!egui::Popup::is_any_open(&harness.ctx));
            harness.read(|editor| assert!(editor.document.look.is_none()));
            harness.read(|editor| assert_eq!(editor.document.project, before));
        }
    });
}

#[test]
fn rendering_widgets_share_builtin_looks_with_views_scopes_and_other_photos_without_regrading() {
    fixture_editor(|mut harness, _| {
        let before = harness.read(|editor| editor.document.project.clone());
        let original = harness.read(|editor| editor.working.displayed.as_ref().unwrap().clone());
        for builtin in Builtin::ALL {
            choose(&mut harness, builtin.name());
            paint_until(&mut harness, |editor| {
                positive_ready(editor)
                    && editor
                        .document
                        .project
                        .output
                        .look
                        .as_ref()
                        .is_some_and(|asset| asset.origin() == &builtin.origin())
            });
            let asset = harness.read(|editor| editor.document.project.output.look.clone().unwrap());
            assert_eq!(asset.origin(), &builtin.origin());
            assert_eq!(asset.name(), builtin.name());
            assert_eq!(
                harness
                    .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Look")
                    .accesskit_node()
                    .value()
                    .as_deref(),
                Some(builtin.name())
            );
            assert_eq!(asset.lut().size(), DEFAULT_GRID);
            if builtin == Builtin::Kodak2383 {
                let project = harness.read(|editor| editor.document.project.clone());
                let time = harness.ctx.input(|input| input.time);
                harness.input_mut().time = Some(time + 0.2);
                harness.step();
                harness
                    .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Look")
                    .hover();
                harness.step();
                let time = harness.ctx.input(|input| input.time);
                harness.input_mut().time = Some(time + 1.);
                harness.step();
                harness.step();
                assert!(harness.query_by_label("Independent creative interpretation derived from published KODAK VISION Color Print Film 2383 / 3383 data.\nNot affiliated with or endorsed by the manufacturer.").is_some());
                harness.read(|editor| assert_eq!(editor.document.project, project));
            }
            harness.read(|editor| assert!(editor.look_render_failure().is_none()));
            harness.read(|editor| assert_eq!(editor.roll, before.roll));
            harness.read(|editor| {
                assert_eq!(
                    editor.adjustment_layer_settings,
                    before.sources[0].adjustment_layers.as_ref().unwrap().layers[0]
                        .adjustment
                        .photo
                )
            });
            harness.read(|editor| {
                assert!(
                    editor
                        .working
                        .displayed
                        .as_ref()
                        .unwrap()
                        .same_grade_as(&original)
                )
            });
            for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
                harness.update(|editor| editor.target = target);
                paint_until(&mut harness, |editor| {
                    if !positive_ready(editor) {
                        return false;
                    }
                    let input = editor.scope_request().unwrap();
                    let settings = crate::scopes::Settings {
                        target: editor.target,
                        plot: if editor.scopes.vector {
                            crate::scopes::Plot::Vector
                        } else {
                            editor.scopes.waveform
                        },
                        amplification: editor.scopes.amplification,
                    };
                    input
                        .status(editor.scopes.gpu.as_ref().unwrap(), settings)
                        .is_some_and(|counts| counts.is_ok())
                });
                harness.read(|editor| {
                    assert_eq!(
                        editor
                            .document
                            .project
                            .output
                            .look
                            .as_ref()
                            .unwrap()
                            .lut()
                            .id(),
                        asset.lut().id()
                    )
                });
            }
            harness
                .get_by_label("Original-resolution reference")
                .click();
            harness.step();
            paint_until(&mut harness, |editor| {
                assert!(
                    editor.original.refinement_error.is_none(),
                    "{}",
                    editor.status
                );
                positive_ready(editor) && editor.working.reference_ready()
            });
            harness.read(|editor| assert!(editor.working.compare));
            harness.read(|editor| {
                assert!(matches!(
                    editor.original.settled.as_ref().unwrap().2.look_result(),
                    Some(Ok(()))
                ))
            });
            if let Some(output) = std::env::var_os("ANTITONE_UI_CAPTURE") {
                let output = PathBuf::from(output);
                std::fs::create_dir_all(&output).unwrap();
                harness
                    .render()
                    .unwrap()
                    .save(output.join(format!("look-{}-reference-offscreen.png", builtin.id())))
                    .unwrap();
            }
            harness
                .get_by_label("Original-resolution reference")
                .click();
            harness.step();
            paint_until(&mut harness, positive_ready);
        }
        let accepted = harness.read(|editor| editor.document.project.output.look.clone().unwrap());
        harness.get_by_label("roll-1.dng").click();
        harness.step();
        wait_for(&mut harness, |editor| !editor.busy);
        paint_until(&mut harness, positive_ready);
        harness.read(|editor| {
            assert_eq!(
                editor
                    .document
                    .project
                    .output
                    .look
                    .as_ref()
                    .unwrap()
                    .lut()
                    .id(),
                accepted.lut().id()
            )
        });
        harness.get_by_label("View source").click();
        harness.step();
        paint_until(&mut harness, |editor| {
            editor.source
                && editor
                    .working
                    .displayed
                    .as_ref()
                    .is_some_and(|view| view.processing.look_id().is_none() && !view.in_flight())
        });
        harness.read(|editor| assert!(editor.look_render_failure().is_none()));
        harness.read(|editor| {
            assert_eq!(
                editor
                    .document
                    .project
                    .output
                    .look
                    .as_ref()
                    .unwrap()
                    .lut()
                    .id(),
                accepted.lut().id()
            )
        });
        harness.get_by_label("View source").click();
        harness.step();
        paint_until(&mut harness, positive_ready);
        choose(&mut harness, "None");
        paint_until(&mut harness, positive_ready);
        harness.read(|editor| assert!(editor.document.project.output.look.is_none()));
        for (size, name) in [
            (egui::vec2(1100., 800.), "normal"),
            (egui::vec2(800., 600.), "narrow"),
        ] {
            harness.set_size(size);
            harness.run();
            let image = harness.render().unwrap();
            if let Some(output) = std::env::var_os("ANTITONE_UI_CAPTURE") {
                let output = PathBuf::from(output);
                std::fs::create_dir_all(&output).unwrap();
                image
                    .save(output.join(format!("looks-{name}-offscreen.png")))
                    .unwrap();
            }
        }
    });
}
#[test]
fn constant_scene_lut_changes_view_thumbnails_and_scope_contents_and_none_restores_pixels() {
    fn pixels(harness: &Harness<'_, Editor>, capture: &Rc<RefCell<FloatCapture>>) -> Vec<[f32; 4]> {
        let positions = [
            harness.read(|editor| editor.last_layout.unwrap().0.image.center()),
            harness.get_by_label("roll-0.dng").rect().center(),
            harness.get_by_label("roll-1.dng").rect().center(),
        ];
        let capture = capture.borrow();
        positions
            .into_iter()
            .map(|point| {
                capture.pixels
                    [point.y.floor() as usize * capture.size[0] as usize + point.x.floor() as usize]
            })
            .collect()
    }

    fn read_bins(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gate: &crate::gpu::SubmissionGate,
        source: &wgpu::Buffer,
    ) -> Vec<u32> {
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: source.size(),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(source, 0, &staging, 0, source.size());
        let submission = gate.submit(queue, [encoder.finish()]);
        let (send, receive) = std::sync::mpsc::channel();
        staging.map_async(wgpu::MapMode::Read, .., move |result| {
            send.send(result).unwrap()
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(Duration::from_secs(30)),
            })
            .unwrap();
        receive
            .recv_timeout(Duration::from_secs(30))
            .unwrap()
            .unwrap();
        let mapped = staging.get_mapped_range(..).unwrap();
        let values = bytemuck::cast_slice::<u8, u32>(&mapped).to_vec();
        drop(mapped);
        staging.unmap();
        values
    }

    fixture_editor_gpu(|mut harness, capture, device, queue| {
        let source = harness.read(|editor| editor.current_source.unwrap());
        let area = harness.read(|editor| editor.document.visit.ready.as_ref().unwrap().area);
        let repair = harness.update(|editor| {
            editor
                .document
                .project
                .source_mut(source)
                .unwrap()
                .repairs
                .insert(
                    crate::dust_repair::SupportMask::new(
                        [area.x + 64, area.y + 64],
                        [3, 3],
                        vec![true; 9],
                    )
                    .unwrap(),
                    crate::dust_repair::RepairOrigin::Manual,
                    crate::dust_repair::DonorChoice::Offset([-32, 0]),
                )
                .unwrap()
        });
        harness.update(|editor| editor.target = OutputTarget::Srgb);
        harness.update(|editor| editor.scopes.vector = true);
        harness.update(|editor| editor.scopes.amplification = 1);
        let settings = crate::scopes::Settings {
            target: OutputTarget::Srgb,
            plot: crate::scopes::Plot::Vector,
            amplification: 1,
        };
        let ready = |editor: &Editor| {
            positive_ready(editor)
                && editor.document.dust.has_evaluation(source, repair)
                && editor
                    .thumbnails
                    .ready_count(&editor.document, editor.target)
                    == 2
                && editor
                    .scope_request()
                    .ok()
                    .and_then(|input| input.status(editor.scopes.gpu.as_ref().unwrap(), settings))
                    .is_some_and(|result| {
                        result.is_ok_and(|counts| counts.valid > 0 && counts.invalid == 0)
                    })
        };
        paint_until(&mut harness, ready);
        let before = pixels(&harness, &capture);
        let neutral_signature = harness.read(|editor| editor.dust_signature().unwrap());
        let evaluated =
            harness.read(|editor| editor.dust_evaluations(neutral_signature)[0].clone());
        let graded = harness.read(|editor| editor.working.displayed.as_ref().unwrap().clone());
        let scene = DVec3::new(0.18, 0.035, 0.008);
        let encoded = scene.map(|value| {
            if value <= 0.0078125 {
                10.5402377416545 * value + 0.0729055341958355
            } else {
                (value.log2() + 9.72) / 17.52
            }
        });
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("constant.cube");
        let mut cube = String::from(
            "TITLE \"Constant scene colour\"\nLUT_3D_SIZE 2\nDOMAIN_MIN -0.5 -0.5 -0.5\nDOMAIN_MAX 1.1 1.1 1.1\n",
        );
        for _ in 0..8 {
            cube.push_str(&format!(
                "{:.17} {:.17} {:.17}\n",
                encoded.x, encoded.y, encoded.z
            ));
        }
        std::fs::write(&file, cube).unwrap();
        harness.enter(|editor, cx| editor.begin_look(look_ui::Source::Imported(file), cx));
        paint_until(&mut harness, ready);
        harness.read(|editor| {
            assert!(Arc::ptr_eq(
                &evaluated,
                &editor.dust_evaluations(neutral_signature)[0]
            ))
        });
        harness.read(|editor| {
            assert!(graded.same_grade_as(editor.working.displayed.as_ref().unwrap()))
        });

        // A constant scene colour has a known result regardless of the upstream photograph.
        // OCIO is independent of the production GPU lookup and selected-output shaders.
        let linear = crate::rendering::CpuRenderer::new(OutputTarget::Srgb)
            .unwrap()
            .render(&[scene])
            .unwrap()[0];
        let expected = linear
            .to_array()
            .map(|value| OutputTarget::Srgb.encode(f64::from(value)).unwrap());
        for pixel in pixels(&harness, &capture) {
            for c in 0..3 {
                // The composed capture includes the RGBA16Float presentation stage.
                assert!(
                    (f64::from(pixel[c]) - expected[c]).abs() <= 0.001,
                    "{pixel:?} vs {expected:?}"
                );
            }
        }
        assert!(
            before
                .iter()
                .all(|pixel| (0..3).any(|c| (f64::from(pixel[c]) - expected[c]).abs() > 0.02))
        );

        let (expected_bins, _, expected_counts) =
            crate::scopes::reference([1, 1], &[[linear.x, linear.y, linear.z, 1.]], settings)
                .unwrap();
        assert_eq!(expected_counts.outside, 0);
        let (counts, submissions, bins) = harness.read(|editor| {
            let counts = editor
                .scope_request()
                .unwrap()
                .status(editor.scopes.gpu.as_ref().unwrap(), settings)
                .unwrap()
                .unwrap();
            (
                counts,
                editor.submissions.clone(),
                editor.scopes.gpu.as_ref().unwrap().bins.clone(),
            )
        });
        assert_eq!(counts.outside, 0);
        let actual = read_bins(&device, &queue, &submissions, &bins);
        for (actual, expected) in actual.iter().zip(expected_bins) {
            assert_eq!(*actual, expected * counts.valid);
        }

        choose(&mut harness, "None");
        paint_until(&mut harness, ready);
        for (restored, original) in pixels(&harness, &capture).iter().zip(before) {
            for c in 0..3 {
                assert!((restored[c] - original[c]).abs() <= 0.001);
            }
        }
    });
}

#[test]
fn loaded_luts_can_be_compared_after_deleting_files_without_persisting_the_session_list() {
    fixture_editor(|mut harness, _| {
        let directory = tempfile::tempdir().unwrap();
        let baseline = harness.read(|editor| (editor.roll, editor.adjustment_layer_settings));
        let mut assets = Vec::new();
        for index in 0..2 {
            let file = directory.path().join(format!("compare-{index}.cube"));
            let mut cube = String::from(
                "TITLE \"Comparison look\"\nLUT_3D_SIZE 2\nDOMAIN_MIN -0.5 -0.5 -0.5\nDOMAIN_MAX 1 1 1\n",
            );
            for b in 0..2 {
                for g in 0..2 {
                    for r in 0..2 {
                        cube.push_str(&format!(
                            "{} {} {}\n",
                            -0.5 + 1.5 * r as f64 + index as f64 * 0.02,
                            -0.5 + 1.5 * g as f64,
                            -0.5 + 1.5 * b as f64
                        ));
                    }
                }
            }
            std::fs::write(&file, cube).unwrap();
            harness
                .enter(|editor, cx| editor.begin_look(look_ui::Source::Imported(file.clone()), cx));
            paint_until(&mut harness, |editor| {
                positive_ready(editor) && editor.loaded_looks.len() == index + 1
            });
            std::fs::remove_file(file).unwrap();
            assets
                .push(harness.read(|editor| editor.document.project.output.look.clone().unwrap()));
        }
        for (size, name) in [
            (egui::vec2(1100., 800.), "normal"),
            (egui::vec2(800., 600.), "narrow"),
        ] {
            harness.set_size(size);
            harness.run();
            let before = harness.read(|editor| editor.document.project.clone());
            harness
                .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Look")
                .click();
            harness.step();
            harness.step();
            assert!(harness.query_by_label("Comparison look").is_some());
            assert!(harness.query_by_label("Comparison look (2)").is_some());
            if let Some(output) = std::env::var_os("ANTITONE_UI_CAPTURE") {
                harness
                    .render()
                    .unwrap()
                    .save(PathBuf::from(output).join(format!("look-loaded-{name}-offscreen.png")))
                    .unwrap();
            }
            harness.event(egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
            harness.run();
            harness.read(|editor| assert_eq!(editor.document.project, before));
        }
        for (name, expected) in [
            (Builtin::Kodak2383.name(), None),
            ("Comparison look", Some(0)),
            ("None", None),
            ("Comparison look (2)", Some(1)),
            (Builtin::PortraEndura.name(), None),
            ("Comparison look", Some(0)),
        ] {
            choose(&mut harness, name);
            paint_until(&mut harness, positive_ready);
            if let Some(index) = expected {
                harness.read(|editor| {
                    assert_eq!(
                        editor
                            .document
                            .project
                            .output
                            .look
                            .as_ref()
                            .unwrap()
                            .lut()
                            .id(),
                        assets[index].lut().id()
                    )
                });
            } else if name == "None" {
                harness.read(|editor| assert!(editor.document.project.output.look.is_none()));
            } else {
                harness.read(|editor| {
                    assert!(matches!(
                        editor
                            .document
                            .project
                            .output
                            .look
                            .as_ref()
                            .unwrap()
                            .origin(),
                        crate::looks::LookOrigin::Builtin { .. }
                    ))
                });
            }
            harness.read(|editor| assert_eq!(editor.loaded_looks.len(), 2));
            harness.read(|editor| {
                assert_eq!((editor.roll, editor.adjustment_layer_settings), baseline)
            });
        }
        let path = directory.path().join("comparison.json");
        harness.enter(|editor, cx| {
            editor.save_project(path.clone(), crate::project::Save::New, None, cx)
        });
        wait_for(&mut harness, |editor| {
            !editor.busy && editor.document_pending.is_none()
        });
        harness.read(|editor| assert!(!editor.document.dirty()));
        editor_harness_with_thumbnails(true, |mut reopened, _| {
            reopened.run();
            open_project(&mut reopened, path.clone());
            wait_for(&mut reopened, |editor| {
                !editor.busy
                    && editor.document_pending.is_none()
                    && editor.document.visit.ready.is_some()
            });
            reopened.read(|editor| assert!(editor.failure.is_none()));
            reopened.read(|editor| assert_eq!(editor.loaded_looks.len(), 1));
            reopened
                .read(|editor| assert_eq!(editor.loaded_looks[0].lut().id(), assets[0].lut().id()));
            reopened.read(|editor| assert!(!editor.document.dirty()));
            open_project(&mut harness, path);
            wait_for(&mut harness, |editor| {
                !editor.busy
                    && editor.document_pending.is_none()
                    && editor.document.visit.ready.is_some()
            });
            harness.read(|editor| assert!(editor.failure.is_none()));
            harness.read(|editor| assert_eq!(editor.loaded_looks.len(), 2));
            harness.read(|editor| assert!(!editor.document.dirty()));
        });
    });
}
#[test]
fn worker_save_reopen_embeds_both_sources_and_export_retains_its_accepted_snapshot() {
    fixture_editor(|mut harness, _| {
        let directory = tempfile::tempdir().unwrap();
        choose(&mut harness, Builtin::Kodak2383.name());
        paint_until(&mut harness, |editor| {
            positive_ready(editor) && editor.document.project.output.look.is_some()
        });
        editor_harness_with_thumbnails(true, |mut reopened, _| {
            reopened.run();
            for imported in [false, true] {
                if imported {
                    let file = directory.path().join("portable.cube");
                    let mut cube = String::from(
                        "TITLE \"Portable scene look\"\nLUT_3D_SIZE 2\nDOMAIN_MIN -0.5 -0.5 -0.5\nDOMAIN_MAX 1.1 1.1 1.1\n",
                    );
                    for b in 0..2 {
                        for g in 0..2 {
                            for r in 0..2 {
                                cube.push_str(&format!(
                                    "{} {} {}\n",
                                    -0.49 + 1.6 * r as f64,
                                    -0.48 + 1.6 * g as f64,
                                    -0.47 + 1.6 * b as f64
                                ));
                            }
                        }
                    }
                    std::fs::write(&file, cube).unwrap();
                    harness.enter(|editor, cx| {
                        editor.begin_look(look_ui::Source::Imported(file.clone()), cx)
                    });
                    paint_until(&mut harness, |editor| {
                        positive_ready(editor)
                            && editor
                                .document
                                .project
                                .output
                                .look
                                .as_ref()
                                .is_some_and(|look| {
                                    matches!(look.origin(), crate::looks::LookOrigin::Imported)
                                })
                    });
                    std::fs::remove_file(file).unwrap();
                }
                let asset =
                    harness.read(|editor| editor.document.project.output.look.clone().unwrap());
                let path = directory.path().join(if imported {
                    "imported.antitone.json"
                } else {
                    "builtin.antitone.json"
                });
                harness.enter(|editor, cx| {
                    editor.save_project(path.clone(), crate::project::Save::New, None, cx)
                });
                wait_for(&mut harness, |editor| {
                    !editor.busy && editor.document_pending.is_none()
                });
                harness.read(|editor| assert!(editor.failure.is_none()));
                harness.read(|editor| assert!(!editor.document.dirty()));
                open_project(&mut reopened, path);
                wait_for(&mut reopened, |editor| {
                    !editor.busy
                        && editor.document_pending.is_none()
                        && editor.document.visit.ready.is_some()
                });
                reopened.read(|editor| assert!(editor.failure.is_none()));
                reopened.read(|editor| {
                    assert_eq!(
                        editor
                            .document
                            .project
                            .output
                            .look
                            .as_ref()
                            .unwrap()
                            .lut()
                            .id(),
                        asset.lut().id()
                    )
                });
                reopened.read(|editor| assert!(editor.document.look.is_none()));
                reopened.read(|editor| assert!(!editor.document.dirty()));
                if reopened.read(|editor| editor.source) {
                    reopened.get_by_label("View source").click();
                    reopened.step();
                }
                paint_until(&mut reopened, positive_ready);
            }
            let path = directory.path().join("snapshot.tif");
            harness.enter(|editor, cx| {
                editor.export(
                    path.clone(),
                    OutputFormat::Tiff(OutputTarget::Srgb),
                    Overwrite::Refuse,
                    cx,
                )
            });
            harness.update(|editor| editor.clear_look());
            wait_for(&mut harness, |editor| {
                editor.export_operation.pending.is_none()
            });
            harness.read(|editor| assert!(editor.failure.is_none()));
            harness.read(|editor| assert!(editor.document.project.output.look.is_none()));
            let cleared = directory.path().join("snapshot-without-look.tif");
            harness.enter(|editor, cx| {
                editor.export(
                    cleared.clone(),
                    OutputFormat::Tiff(OutputTarget::Srgb),
                    Overwrite::Refuse,
                    cx,
                )
            });
            wait_for(&mut harness, |editor| {
                editor.export_operation.pending.is_none()
            });
            harness.read(|editor| assert!(editor.failure.is_none()));
            assert_ne!(tiff_codes(&path), tiff_codes(&cleared));
        });
    });
}

#[test]
fn narrow_import_clips_finite_values_without_regrading_and_calibration_bypasses_it() {
    fixture_editor(|mut harness, _| {
        let baseline = harness.read(|editor| (editor.roll, editor.adjustment_layer_settings));
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("narrow.cube");
        let mut cube = String::from(
            "TITLE \"Narrow scene domain\"\nLUT_3D_SIZE 2\nDOMAIN_MIN 0.99 0.99 0.99\nDOMAIN_MAX 1 1 1\n",
        );
        for _ in 0..8 {
            cube.push_str("0.4 0.4 0.4\n");
        }
        std::fs::write(&path, cube).unwrap();
        harness.enter(|editor, cx| editor.begin_look(look_ui::Source::Imported(path.clone()), cx));
        wait_for(&mut harness, |editor| !editor.busy);
        std::fs::remove_file(path).unwrap();
        let accepted = harness.read(|editor| {
            editor
                .document
                .project
                .output
                .look
                .as_ref()
                .unwrap()
                .lut()
                .id()
        });
        paint_until(&mut harness, positive_ready);
        harness.read(|editor| assert!(editor.look_render_failure().is_none()));
        harness
            .read(|editor| assert_eq!((editor.roll, editor.adjustment_layer_settings), baseline));
        harness.read(|editor| {
            assert_eq!(
                editor
                    .document
                    .project
                    .output
                    .look
                    .as_ref()
                    .unwrap()
                    .lut()
                    .id(),
                accepted
            )
        });
        assert!(harness.query_by_label("Look rendering failed").is_none());
        harness.enter(|editor, cx| editor.begin_pick(Pick::Gamma, cx));
        harness.step();
        paint_until(&mut harness, |editor| {
            assert!(
                editor.original.refinement_error.is_none(),
                "{}",
                editor.status
            );
            editor
                .original
                .signature
                .as_ref()
                .is_some_and(|signature| signature.0.look_id().is_none())
                && editor.original.settled.is_some()
        });
        harness.read(|editor| assert!(editor.look_render_failure().is_none()));
        harness.read(|editor| {
            assert_eq!(
                editor
                    .document
                    .project
                    .output
                    .look
                    .as_ref()
                    .unwrap()
                    .lut()
                    .id(),
                accepted
            )
        });
        harness.get_by_label("Cancel").click();
        harness.step();
        paint_until(&mut harness, |editor| {
            editor.pick.is_none() && editor.document.pick_return.is_none() && positive_ready(editor)
        });
        harness.read(|editor| assert!(editor.look_render_failure().is_none()));
        harness
            .read(|editor| assert_eq!((editor.roll, editor.adjustment_layer_settings), baseline));
        harness.read(|editor| {
            assert_eq!(
                editor
                    .document
                    .project
                    .output
                    .look
                    .as_ref()
                    .unwrap()
                    .lut()
                    .id(),
                accepted
            )
        });
        choose(&mut harness, "None");
        paint_until(&mut harness, positive_ready);
        harness.read(|editor| assert!(editor.look_render_failure().is_none()));
        harness.read(|editor| assert!(editor.document.project.output.look.is_none()));
    });
}
