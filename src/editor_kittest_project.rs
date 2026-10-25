use super::workflow::{enter_numeric, wait_settled};
use super::*;

fn compare_pixel(harness: &Harness<'_, Editor>) -> (DVec3, Vec<glam::Vec3>, Vec<glam::Vec3>) {
    let (path, wb, input, roll, photo) = harness.read(|editor| {
        let document = &editor.document;
        let id = editor.current_source.unwrap();
        (
            document.source_path(id).unwrap(),
            document.project.development().as_ref().unwrap().fixed_wb,
            document.project.input,
            document.project.roll,
            document.project.source(id).unwrap().photo,
        )
    });
    let scan = Scan::open(&path, Some(wb)).unwrap();
    let pixel = scan
        .pixel(
            scan.area.x + scan.area.width / 2,
            scan.area.y + scan.area.height / 2,
        )
        .unwrap();
    let working = crate::processing::Pipeline::new(input, roll, photo, scan.camera_to_ap1)
        .unwrap()
        .apply(pixel)
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
                    editor.input = crate::processing::InputSettings::Digital;
                }
                editor.roll.wb = Balance {
                    warmth: 0.2,
                    tint: -0.15,
                };
                editor.sync_document();
            });
            let input = harness.read(|editor| editor.input);
            stage(&mut harness, "Roll");
            reset_shared_balance_component(&mut harness, "Warmth", 0.2);
            harness.read(|editor| {
                assert_eq!(
                    editor.roll.wb,
                    Balance {
                        warmth: 0.,
                        tint: -0.15
                    }
                );
                assert_eq!(editor.input, input);
                assert_eq!(editor.document.project.roll, editor.roll);
            });
            reset_shared_balance_component(&mut harness, "Tint", -0.15);
            harness.read(|editor| {
                assert_eq!(editor.roll.wb, Balance::default());
                assert_eq!(editor.input, input);
            });
        });
    }
}

#[test]
fn interpretation_numeric_edits_reject_unsupported_processing_before_accepting_the_document() {
    tools::calibrated_harness(|mut harness| {
        harness.update(|editor| {
            let crate::processing::InputSettings::Negative { response, .. } = &mut editor.input
            else {
                panic!("calibrated negative fixture");
            };
            *response = crate::processing::Response::Film {
                toe: 2.5,
                shoulder: 0.02,
            };
            editor.sync_document();
            editor
                .validate_processing(editor.input, editor.roll, editor.photo)
                .unwrap();
        });
        let before = harness.read(|editor| editor.document.project.clone());
        stage(&mut harness, "Roll");
        enter_numeric(&mut harness, "Gamma", "0.1");
        harness.read(|editor| {
            assert!(
                editor.status.starts_with("Edit rejected:"),
                "{}",
                editor.status
            );
            assert_eq!(editor.document.project, before);
            assert_eq!(editor.input, before.input);
        });
        enter_numeric(&mut harness, "Gamma", "0.7");
        harness.read(|editor| {
            assert_eq!(editor.input.gamma(), 0.7);
            assert_eq!(editor.document.project.input, editor.input);
        });
    });
}

#[test]
fn project_widgets_save_precision_order_wb_and_reject_open_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let paths: Vec<_> = ["calibration.dng", "working.dng", "unvisited.dng"]
        .map(|name| {
            let path = directory.path().join(name);
            std::fs::copy(fixture("patches.dng"), &path).unwrap();
            path
        })
        .into();
    editor_harness_with_thumbnails(true, |mut harness, _| {
        let custom = [2., 1., 1.5, 1.];
        harness.update(|editor| {
            editor.document.project.scan = Some(scan_setup(&paths[0], custom));
        });
        harness.enter(|editor, cx| editor.add_raws(paths.clone(), cx));
        wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
        harness.read(|editor| {
            assert!(editor.document.visit.ready.is_some(), "{}", editor.status);
            assert!(
                editor.document.project.sources.iter().all(|source| source
                    .development
                    .decode
                    .fixed_wb
                    == custom)
            );
        });
        let mut scan = Scan::open(&paths[0], Some(custom)).unwrap();
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
            e.source = false;
            e.sync_document();
        });
        let ids = harness.read(|editor| editor.document.project.order.clone());
        harness.step();
        harness.get_by_label("working.dng").click();
        harness.step();
        wait_for(&mut harness, |e| !e.busy);
        harness.read(|editor| assert_eq!(editor.current_source, Some(ids[1])));
        stage(&mut harness, "Photo");
        enter_numeric(&mut harness, "Exposure", "7.123456789012345");
        harness.read(|editor| assert_eq!(editor.photo.exposure, 7.123456789012345));
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
                crate::processing::Response::Film {
                    toe: 2.789123456789,
                    shoulder: 0.003123456789
                }
            )
        });
        // Supply a reordered roll to exercise source-owned Save/Open behavior.
        harness.update(|e| {
            e.document.project.order.swap(0, 1);
            e.selected = 0;
            e.sync_document();
            let path = directory.path().join("film.antitone");
            *e.document = e.document.save(&path, crate::project::Save::New).unwrap();
        });
        harness.read(|editor| {
            assert_eq!(
                editor.document.project.source(ids[2]).unwrap().photo,
                PhotoSettings::default()
            )
        });
        stage(&mut harness, "Photo");
        enter_numeric(&mut harness, "Warmth", "0.1");
        harness.read(|editor| assert!(editor.document.dirty()));
        let expected = harness.read(|editor| editor.document.project.clone());
        let pixel = compare_pixel(&harness);
        save_existing_project(&mut harness);
        file_menu(&mut harness, "New");
        wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
        harness.read(|editor| assert!(editor.document.project.development().is_none()));
        harness.read(|editor| assert!(editor.document.visit.ready.is_none()));
        harness.enter(|editor, cx| editor.add_raws(vec![paths[0].clone()], cx));
        wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
        let as_shot = crate::raw::RawFile::open(&paths[0])
            .unwrap()
            .metadata
            .as_shot_wb();
        harness.read(|editor| {
            assert_eq!(
                editor
                    .document
                    .project
                    .development()
                    .as_ref()
                    .unwrap()
                    .fixed_wb,
                as_shot
            )
        });
        assert_ne!(as_shot, custom);
        let before_open = harness.read(|editor| editor.document.project.clone());
        open_project(&mut harness, directory.path().join("film.antitone"));
        harness.read(|editor| assert!(editor.transition.is_some()));
        harness.get_by_label("Cancel").click();
        harness.step();
        harness.read(|editor| assert_eq!(editor.document.project, before_open));
        open_project(&mut harness, directory.path().join("film.antitone"));
        harness.get_by_label("Discard changes").click();
        harness.step();
        wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
        harness.read(|editor| assert_eq!(editor.document.project, expected));
        harness.read(|editor| assert!(!editor.document.dirty()));
        harness.read(|editor| assert_eq!(editor.current_source, Some(ids[1])));
        harness.read(|editor| {
            assert_eq!(
                editor
                    .document
                    .visit
                    .ready
                    .as_ref()
                    .unwrap()
                    .development()
                    .fixed_wb,
                custom
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
            harness.update(|editor| editor.selection_timing = Some(timing.clone()));
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
        let mut classic = harness.read(|editor| editor.document.project.clone());
        classic.set_response(crate::processing::Response::Original);
        let classic_path = directory.path().join("classic.antitone.json");
        classic
            .save(&classic_path, crate::project::Save::New)
            .unwrap();
        open_project(&mut harness, classic_path);
        wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
        harness.read(|editor| assert_eq!(editor.document.project, classic));
        let classic_pixel = compare_pixel(&harness);
        stage(&mut harness, "Photo");
        enter_numeric(&mut harness, "Tint", "-0.05");
        harness.read(|editor| classic = editor.document.project.clone());
        let classic_pixel_edited = compare_pixel(&harness);
        assert_ne!(classic_pixel_edited, classic_pixel);
        save_existing_project(&mut harness);
        open_project(&mut harness, directory.path().join("classic.antitone.json"));
        wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
        harness.read(|editor| assert!(editor.response().is_original()));
        harness.read(|editor| assert_eq!(editor.document.project, classic));
        harness.read(|editor| assert!(!editor.document.dirty()));
        wait_settled(&mut harness);
        assert_eq!(compare_pixel(&harness), classic_pixel_edited);
        let selected = harness.read(|editor| editor.current_source);
        let ready =
            harness.read(|editor| editor.document.visit.ready.as_ref().unwrap().proxy.clone());
        for (name, id, missing) in [
            ("changed", ids[1], false),
            ("missing", ids[1], true),
            ("incompatible", ids[2], false),
        ] {
            let mut rejected = classic.clone();
            if missing {
                rejected.source_mut(id).unwrap().path = directory.path().join("missing.dng");
            } else {
                rejected.source_mut(id).unwrap().development.dimensions[0] += 1;
            }
            let path = directory.path().join(format!("{name}.antitone.json"));
            rejected.save(&path, crate::project::Save::New).unwrap();
            open_project(&mut harness, path);
            wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
            harness.read(|editor| {
                assert!(
                    editor.status.contains(if missing {
                        "missing.dng"
                    } else {
                        "development identity differs"
                    }),
                    "{}",
                    editor.status
                )
            });
            harness.read(|editor| assert_eq!(editor.document.project, classic));
            harness.read(|editor| assert_eq!(editor.current_source, selected));
            harness.read(|editor| {
                assert!(
                    editor
                        .document
                        .visit
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
            e.thumbnails.ready_count(&e.document, e.target)
                >= e.document.project.order.len().min(16)
        });
        harness.read(|editor| assert_eq!(editor.document.project, classic));
        stage(&mut harness, "Photo");
        enter_numeric(&mut harness, "Exposure", "0.5");
        let dirty = harness.read(|editor| editor.document.project.clone());
        let saved_path = harness.read(|editor| editor.document.path.clone());
        let failed = directory.path().join("unwritable.antitone.json");
        std::fs::create_dir(&failed).unwrap();
        harness.update(|editor| editor.document.path = Some(failed));
        file_menu(&mut harness, "New");
        harness.step();
        harness.read(|editor| assert!(editor.transition.is_some()));
        let request = harness.read(|editor| editor.next_request);
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Save changes")
            .click();
        harness.step();
        harness.read(|editor| {
            assert!(
                editor.next_request > request,
                "transition Save did not dispatch"
            )
        });
        wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
        harness.read(|editor| assert_eq!(editor.document.project, dirty));
        harness.read(|editor| assert!(editor.document.dirty()));
        harness.read(|editor| assert_eq!(editor.transition, Some(files::Transition::New)));
        harness.read(|editor| assert!(editor.document.visit.ready.is_some()));
        harness.read(|editor| assert_eq!(editor.failure.as_ref().unwrap().kind, FailureKind::Save));
        harness.get_by_label("OK").click();
        harness.step();
        harness.step();
        harness.read(|editor| assert!(editor.failure.is_none()));
        harness.read(|editor| assert_eq!(editor.document.project, dirty));
        harness.get_by_label("Cancel").click();
        harness.step();
        harness.update(|editor| editor.document.path = saved_path);
        harness.read(|editor| assert!(editor.transition.is_none()));
        harness.read(|editor| assert!(editor.preference_store.is_none()));
    });
}
