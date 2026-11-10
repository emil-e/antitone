use super::workflow::{enter_numeric, wait_settled};
use super::*;

#[test]
fn adding_photos_updates_the_filmstrip_without_revisiting_the_active_photo() {
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first.dng");
    let added = directory.path().join("added.dng");
    for path in [&first, &added] {
        std::fs::copy(fixture("patches.dng"), path).unwrap();
    }
    editor_harness_with_thumbnails(true, |mut harness, _| {
        // Known chooser results enter the production Import workflow; no native picker is opened.
        harness.enter(|editor, cx| editor.add_raws(vec![first.clone()], cx));
        wait_opened_photo(&mut harness);
        harness.try_run_realtime().unwrap();
        let (selected, view, pixels) = harness.read(|editor| {
            (
                editor.document.visit.current_source,
                editor.document.visit.view,
                editor
                    .document
                    .visit
                    .canvas
                    .ready
                    .as_ref()
                    .unwrap()
                    .developed
                    .clone(),
            )
        });
        harness.enter(|editor, cx| {
            editor.add_raws(vec![first.clone(), added.clone(), added.clone()], cx)
        });
        wait_for(&mut harness, |editor| editor.document.importing.is_none());
        harness.try_run_realtime().unwrap();
        harness.read(|editor| {
            assert_eq!(editor.document.project.sources.len(), 2);
            assert_eq!(editor.document.visit.current_source, selected);
            assert_eq!(editor.document.visit.view, view);
            assert!(Arc::ptr_eq(
                &editor
                    .document
                    .visit
                    .canvas
                    .ready
                    .as_ref()
                    .unwrap()
                    .developed,
                &pixels
            ));
        });
        assert_eq!(
            harness.get_by_label("first.dng").accesskit_node().toggled(),
            Some(egui::accesskit::Toggled::True)
        );
        assert_eq!(
            harness.get_by_label("added.dng").accesskit_node().toggled(),
            Some(egui::accesskit::Toggled::False)
        );
        let accepted = harness.read(|editor| editor.document.project.clone());
        harness.enter(|editor, cx| editor.add_raws(vec![first, added], cx));
        wait_for(&mut harness, |editor| editor.document.importing.is_none());
        harness.read(|editor| assert_eq!(editor.document.project, accepted));
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        harness.step();
        wait_opened_photo(&mut harness);
        harness.read(|editor| {
            assert_eq!(
                editor.document.project.sources.len(),
                1,
                "duplicate-only import must not create an undo entry"
            );
            assert_eq!(editor.document.visit.current_source, selected);
        });
        assert!(harness.query_by_label("added.dng").is_none());
    });
}

fn compare_pixel(harness: &Harness<'_, Editor>) -> (DVec3, Vec<glam::Vec3>, Vec<glam::Vec3>) {
    let (path, wb, input, roll, snapshot) = harness.read(|editor| {
        let document = &editor.document;
        let id = editor.document.visit.current_source.unwrap();
        (
            document.source_path(id).unwrap(),
            document.project.acquisition_wb(),
            document.project.input,
            document.project.roll,
            document
                .project
                .source(id)
                .unwrap()
                .grade_snapshot()
                .unwrap(),
        )
    });
    let scan = Scan::open(&path, wb).unwrap();
    let point = [
        scan.area.x + scan.area.width / 2,
        scan.area.y + scan.area.height / 2,
    ];
    let pixel = scan.pixel(point[0], point[1]).unwrap();
    let working = crate::adjustment_layers::AdjustmentLayerPipeline::new(
        input,
        roll,
        snapshot,
        scan.camera_to_ap1,
    )
    .unwrap()
    .apply(pixel, point)
    .unwrap()
    .rgb;
    (
        working,
        crate::rendering::CpuRenderer::new(OutputTarget::Srgb)
            .unwrap()
            .render(&[working])
            .unwrap(),
        crate::rendering::CpuRenderer::new(OutputTarget::DisplayP3)
            .unwrap()
            .render(&[working])
            .unwrap(),
    )
}

fn reset_shared_balance_component(harness: &mut Harness<'_, Editor>, label: &str, before: f64) {
    for click in [false, true] {
        // Gamma balance has the same axis labels. The fixture changes only photographic WB.
        let mut controls = harness
            .get_all_by_role_and_label(egui::accesskit::Role::SpinButton, label)
            .filter(|node| node.accesskit_node().numeric_value() == Some(before));
        let control = controls.next().expect("changed shared WB component");
        assert!(controls.next().is_none(), "only shared WB has this value");
        if click {
            control.click_secondary();
        } else {
            control.scroll_to_me();
        }
        drop(controls);
        harness.try_run_realtime().unwrap();
    }
    harness.get_by_label("Reset to default").click();
    harness.step();
}

#[test]
fn individual_roll_white_balance_resets_work_in_both_input_stages() {
    for digital in [false, true] {
        tools::calibrated_harness(|mut harness| {
            harness.update(|editor| {
                if digital {
                    editor.document.project.input = crate::processing::InputSettings::Digital;
                }
                editor.document.project.roll.wb = Balance {
                    warmth: 0.2,
                    tint: -0.15,
                };
            });
            let input = harness.read(|editor| editor.document.project.input);
            stage(&mut harness, "Roll");
            reset_shared_balance_component(&mut harness, "Warmth", 0.2);
            harness.read(|editor| {
                assert_eq!(
                    editor.document.project.roll.wb,
                    Balance {
                        warmth: 0.,
                        tint: -0.15
                    }
                );
                assert_eq!(editor.document.project.input, input);
            });
            reset_shared_balance_component(&mut harness, "Tint", -0.15);
            harness.read(|editor| {
                assert_eq!(editor.document.project.roll.wb, Balance::default());
                assert_eq!(editor.document.project.input, input);
            });
        });
    }
}

#[test]
fn low_gamma_with_shoulder_is_usable_and_numeric_edits_update_the_document() {
    tools::calibrated_harness(|mut harness| {
        harness.update(|editor| {
            let crate::processing::InputSettings::Negative { response, .. } =
                &mut editor.document.project.input
            else {
                panic!("calibrated negative fixture");
            };
            *response = crate::processing::Response {
                toe: 2.5,
                shoulder: 0.02,
            };
            editor
                .validate_processing(
                    editor.document.project.input,
                    editor.document.project.roll,
                    editor.photo_settings(),
                )
                .unwrap();
        });
        stage(&mut harness, "Roll");
        enter_numeric(&mut harness, "Gamma", "0.1");
        harness.read(|editor| {
            assert!(
                !editor.status.starts_with("Edit rejected:"),
                "{}",
                editor.status
            );
            assert_eq!(editor.document.project.input.gamma(), 0.1);
        });
        enter_numeric(&mut harness, "Gamma", "0.7");
        harness.read(|editor| {
            assert_eq!(editor.document.project.input.gamma(), 0.7);
        });
    });
}

#[test]
fn project_widgets_save_precision_order_and_reject_invalid_project_metadata_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let paths: Vec<_> = ["calibration.dng", "working.dng", "unvisited.dng"]
        .map(|name| {
            let path = directory.path().join(name);
            std::fs::copy(fixture("patches.dng"), &path).unwrap();
            path
        })
        .into();
    editor_harness_with_thumbnails(true, |mut harness, _| {
        harness.enter(|editor, cx| editor.add_raws(paths.clone(), cx));
        wait_opened_photo(&mut harness);
        harness.read(|editor| {
            assert!(
                editor.document.visit.canvas.ready.is_some(),
                "{}",
                editor.status
            );
            assert!(editor.document.state.project.sources.iter().all(|source| {
                Scan::open(
                    &editor.document.source_path(source.id).unwrap(),
                    editor.document.project.acquisition_wb(),
                )
                .unwrap()
                .white_balance
                    == crate::project::UNITY_WB
            }));
        });
        let mut scan = Scan::open(&paths[0], Some(crate::project::UNITY_WB)).unwrap();
        harness.update(|e| {
            e.set_dmin(Some(
                scan.reference(Rect {
                    x: 16,
                    y: 16,
                    width: 32,
                    height: 32,
                })
                .unwrap(),
            ));
            e.set_dmax(Some(
                scan.reference(Rect {
                    x: 80,
                    y: 16,
                    width: 32,
                    height: 32,
                })
                .unwrap(),
            ));
            e.document.source = false;
        });
        let ids = harness.read(|editor| editor.document.state.project.order.clone());
        harness.step();
        harness.get_by_label("working.dng").click();
        harness.step();
        wait_for(&mut harness, |e| !e.interaction_blocked());
        harness.read(|editor| assert_eq!(editor.document.visit.current_source, Some(ids[1])));
        stage(&mut harness, "Photo");
        enter_numeric(&mut harness, "Exposure", "7.123456789012345");
        harness.read(|editor| assert_eq!(editor.photo_settings().exposure, 7.123456789012345));
        stage(&mut harness, "Roll");
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Toe & shoulder")
            .scroll_to_me();
        harness.try_run_realtime().unwrap();
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Toe & shoulder")
            .click();
        harness.step();
        enter_numeric(&mut harness, "Toe", "2.789123456789");
        enter_numeric(&mut harness, "Shoulder", "0.003123456789");
        harness.read(|editor| {
            assert_eq!(
                editor.response(),
                crate::processing::Response {
                    toe: 2.789123456789,
                    shoulder: 0.003123456789
                }
            )
        });
        // Supply a reordered roll to exercise source-owned Save/Open behavior.
        harness.update(|e| {
            e.document.state.project.order.swap(0, 1);
            e.document.selected = 0;
            let path = directory.path().join("film.antitone");
            *e.document = e.document.save(&path, crate::project::Save::New).unwrap();
        });
        harness.read(|editor| {
            assert_eq!(
                editor
                    .document
                    .project
                    .source(ids[2])
                    .unwrap()
                    .adjustment_layers
                    .as_ref()
                    .unwrap()
                    .layers[0]
                    .adjustment
                    .photo,
                AdjustmentLayerSettings::default()
            )
        });
        stage(&mut harness, "Photo");
        enter_numeric(&mut harness, "Warmth", "0.1");
        harness.read(|editor| assert!(editor.document.dirty()));
        let expected = harness.read(|editor| editor.document.state.project.clone());
        let pixel = compare_pixel(&harness);
        save_existing_project(&mut harness);
        native_file_action(&mut harness, crate::platform::menu::Action::New);
        wait_for(&mut harness, |e| {
            !e.interaction_blocked() && e.document_pending.is_none()
        });
        harness.read(|editor| assert!(editor.document.state.project.development().is_none()));
        harness.read(|editor| assert!(editor.document.visit.canvas.ready.is_none()));
        harness.enter(|editor, cx| editor.add_raws(vec![paths[0].clone()], cx));
        wait_opened_photo(&mut harness);
        harness.read(|editor| {
            assert_eq!(
                editor
                    .document
                    .visit
                    .canvas
                    .ready
                    .as_ref()
                    .unwrap()
                    .developed
                    .white_balance(),
                crate::project::UNITY_WB
            )
        });
        let before_open = harness.read(|editor| editor.document.state.project.clone());
        open_project(&mut harness, directory.path().join("film.antitone"));
        harness.read(|editor| assert!(editor.files.pending()));
        harness.get_by_label("Cancel").click();
        harness.step();
        wait_for(&mut harness, |editor| !editor.files.pending());
        harness.read(|editor| assert_eq!(editor.document.state.project, before_open));
        open_project(&mut harness, directory.path().join("film.antitone"));
        harness.get_by_label("Discard changes").click();
        harness.step();
        wait_opened_photo(&mut harness);
        harness.read(|editor| assert_eq!(editor.document.state.project, expected));
        harness.read(|editor| assert!(!editor.document.dirty()));
        harness.read(|editor| assert_eq!(editor.document.visit.current_source, Some(ids[1])));
        harness.read(|editor| {
            assert_eq!(
                editor
                    .document
                    .visit
                    .canvas
                    .ready
                    .as_ref()
                    .unwrap()
                    .developed
                    .white_balance(),
                crate::project::UNITY_WB
            )
        });
        wait_settled(&mut harness);
        assert_eq!(compare_pixel(&harness), pixel);
        stage(&mut harness, "Rendering");
        for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
            select_target(&mut harness, target);
            wait_settled(&mut harness);
            let timing =
                harness.read(|editor| Arc::new(SelectionTiming::new(editor.id, Instant::now())));
            harness.update(|editor| {
                editor.document.visit.canvas.selection_timing = Some(timing.clone())
            });
            harness.step();
            let png = harness.render().unwrap();
            assert!(timing.ready_was_painted());
            save_capture(
                &png,
                if target == OutputTarget::Srgb {
                    "project-film-srgb-offscreen"
                } else {
                    "project-film-p3-offscreen"
                },
            );
        }
        save_existing_project(&mut harness);
        let mut adjusted = harness.read(|editor| editor.document.state.project.clone());
        adjusted.set_response(crate::processing::Response {
            toe: 3.25,
            shoulder: 0.004,
        });
        let adjusted_path = directory.path().join("adjusted.antitone.json");
        adjusted
            .save(&adjusted_path, crate::project::Save::New)
            .unwrap();
        open_project(&mut harness, adjusted_path);
        wait_opened_photo(&mut harness);
        harness.read(|editor| assert_eq!(editor.document.state.project, adjusted));
        let adjusted_pixel = compare_pixel(&harness);
        stage(&mut harness, "Photo");
        enter_numeric(&mut harness, "Tint", "-0.05");
        harness.read(|editor| adjusted = editor.document.state.project.clone());
        let adjusted_pixel_edited = compare_pixel(&harness);
        assert_ne!(adjusted_pixel_edited, adjusted_pixel);
        save_existing_project(&mut harness);
        open_project(
            &mut harness,
            directory.path().join("adjusted.antitone.json"),
        );
        wait_opened_photo(&mut harness);
        harness.read(|editor| assert_eq!(editor.document.state.project, adjusted));
        harness.read(|editor| assert!(!editor.document.dirty()));
        wait_settled(&mut harness);
        assert_eq!(compare_pixel(&harness), adjusted_pixel_edited);
        let selected = harness.read(|editor| editor.document.visit.current_source);
        let ready = harness.read(|editor| {
            editor
                .document
                .visit
                .canvas
                .ready
                .as_ref()
                .unwrap()
                .proxy
                .clone()
        });
        let mut unsupported = serde_json::to_value(&adjusted).unwrap();
        unsupported["schema"] = serde_json::json!(u32::MAX);
        for (name, bytes) in [
            ("malformed", b"{".to_vec()),
            (
                "unsupported-schema",
                serde_json::to_vec(&unsupported).unwrap(),
            ),
        ] {
            let path = directory.path().join(format!("{name}.antitone"));
            std::fs::write(&path, bytes).unwrap();
            assert!(crate::project::Project::load(&path).is_err());
            open_project(&mut harness, path);
            wait_opened_photo(&mut harness);
            harness.read(|editor| assert_eq!(editor.document.state.project, adjusted));
            harness.read(|editor| assert_eq!(editor.document.visit.current_source, selected));
            harness.read(|editor| {
                assert!(
                    editor
                        .document
                        .visit
                        .canvas
                        .ready
                        .as_ref()
                        .unwrap()
                        .proxy
                        .same_source(&ready)
                )
            });
            harness.read(|editor| assert!(!editor.document.dirty()));
        }
        wait_for(&mut harness, |e| {
            e.document
                .thumbnails
                .ready_count(&e.document, e.document.target())
                >= e.document.state.project.order.len().min(16)
        });
        harness.read(|editor| assert_eq!(editor.document.state.project, adjusted));
        stage(&mut harness, "Photo");
        enter_numeric(&mut harness, "Exposure", "0.5");
        let dirty = harness.read(|editor| editor.document.state.project.clone());
        let saved_path = harness.read(|editor| editor.document.path.clone());
        let failed = directory.path().join("unwritable.antitone.json");
        std::fs::create_dir(&failed).unwrap();
        harness.update(|editor| editor.document.path = Some(failed));
        native_file_action(&mut harness, crate::platform::menu::Action::New);
        harness.try_run_realtime().unwrap();
        harness.read(|editor| assert!(editor.files.pending()));
        let request = harness.read(|editor| editor.next_request);
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Save changes")
            .click();
        harness.step();
        wait_for(&mut harness, |editor| editor.next_request > request);
        wait_for(&mut harness, |e| {
            !e.interaction_blocked() && e.document_pending.is_none()
        });
        harness.read(|editor| assert_eq!(editor.document.state.project, dirty));
        harness.read(|editor| assert!(editor.document.dirty()));
        harness.read(|editor| assert!(editor.files.pending()));
        harness.read(|editor| assert!(editor.document.visit.canvas.ready.is_some()));
        harness.read(|editor| {
            assert_eq!(
                editor.dialogs.message().unwrap().title,
                FailureCase::Save.title()
            )
        });
        harness.get_by_label("OK").click();
        harness.step();
        wait_for(&mut harness, |editor| editor.dialogs.message().is_none());
        harness.read(|editor| assert!(editor.dialogs.is_open() && editor.files.pending()));
        harness.read(|editor| assert_eq!(editor.document.state.project, dirty));
        harness.get_by_label("Cancel").click();
        harness.step();
        wait_for(&mut harness, |editor| !editor.files.pending());
        harness.update(|editor| editor.document.path = saved_path);
        harness.read(|editor| assert!(!editor.files.pending()));
        harness.read(|editor| assert!(editor.preference_store.is_none()));
    });
}
