use super::*;
use crate::gpu::parameters::ProcessingParameters;
use crate::raw::sony_fixture as sony;

#[test]
fn opened_missing_photo_keeps_its_card_error_and_another_card_can_be_selected() {
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("missing.dng");
    let available = directory.path().join("available.dng");
    for path in [&missing, &available] {
        std::fs::copy(fixture("patches.dng"), path).unwrap();
    }
    let mut document = Document::default();
    document
        .import(&[missing.clone(), available.clone()])
        .unwrap();
    let saved = document
        .save(
            &directory.path().join("missing-photo.antitone"),
            crate::project::Save::New,
        )
        .unwrap();
    std::fs::remove_file(missing).unwrap();
    editor_harness_with_thumbnails(true, |mut harness, _| {
        harness.enter(|editor, cx| editor.open_project(saved.path.clone().unwrap(), cx));
        wait_for(&mut harness, |editor| {
            !editor.files.opening() && !editor.document.visit.source.pending()
        });
        let deadline = Instant::now() + Duration::from_secs(15);
        while harness.query_by_label("Photo error").is_none() {
            assert!(
                Instant::now() < deadline,
                "missing photo must report an error on its card"
            );
            harness.step();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(harness.output().shapes.iter().any(|shape| matches!(
            &shape.shape, egui::Shape::Text(text) if text.galley.job.text == "Photo unavailable"
        )));
        assert!(
            harness
                .get_by_label("missing.dng")
                .accesskit_node()
                .description()
                .unwrap()
                .contains("Thumbnail unavailable")
        );
        harness.read(|editor| {
            assert_eq!(editor.document.state.project, saved.project);
            assert_eq!(
                editor.document.visit.current_source,
                Some(saved.project.order[0])
            );
            assert!(editor.document.visit.canvas.ready.is_none());
            assert!(!editor.document.dirty() && !editor.dialogs.is_open());
        });
        save_capture(&harness.render().unwrap(), "open-missing-photo-offscreen");
        harness.get_by_label("available.dng").click();
        harness.step();
        wait_for(&mut harness, |editor| {
            !editor.document.visit.source.pending() && editor.document.visit.canvas.ready.is_some()
        });
        harness.read(|editor| {
            assert_eq!(
                editor.document.visit.current_source,
                Some(saved.project.order[1])
            );
            assert_eq!(editor.document.state.project, saved.project);
            assert!(!editor.document.dirty());
        });
    });
}

#[test]
fn digital_source_scale_import_save_open_and_inspection_keep_zero_exposure_controls() {
    let file = sony::source_with_as_shot(sony::Order::Little, vec![], [4, 1, 2]);
    let directory = tempfile::tempdir().unwrap();
    editor_harness_with_thumbnails(true, |mut h, _| {
        h.update(|editor| {
            editor.document.project.input = crate::processing::InputSettings::Digital;
        });
        h.enter(|editor, cx| editor.add_raws(vec![file.path().into()], cx));
        wait_for(&mut h, |e| {
            !e.interaction_blocked()
                && e.document.visit.working_view.support_ready()
                && e.document
                    .thumbnails
                    .ready_count(&e.document, e.document.target())
                    == 1
        });
        h.render().unwrap();
        let accepted = h.read(|editor| {
            assert!(!editor.document.source);
            assert_eq!(editor.document.project.roll.exposure, 0.);
            assert_eq!(editor.photo_settings().exposure, 0.);
            let ready = editor.document.visit.canvas.ready.as_ref().unwrap();
            assert_eq!(ready.developed.white_balance(), [4., 1., 2., 1.]);
            assert!(!ready.record.development.decode.uses_sdk());
            assert_eq!(
                ready.developed.precision_floor(),
                4. * crate::processing::CAMERA_INPUT_FLOOR
            );
            editor.document.state.project.clone()
        });
        h.get_by_label("View source").click();
        h.step();
        wait_for(&mut h, |e| {
            e.document.visit.working_view.support_ready()
                && e.document
                    .visit
                    .working_view
                    .displayed
                    .as_ref()
                    .is_some_and(|v| v.spec.is_inspection())
        });
        h.render().unwrap();
        h.read(|editor| {
            assert!(editor.document.source);
            // Inspection retains only its separate +1 EV convention, not another x4.
            assert_eq!(
                bytemuck::bytes_of(&editor.source_parameters(0.).unwrap()),
                bytemuck::bytes_of(&ProcessingParameters::source_preview(None, 0.).unwrap())
            );
            let record = editor
                .document
                .project
                .source(editor.document.visit.current_source.unwrap())
                .unwrap();
            assert_eq!(editor.document.state.project, accepted);
            let saved = editor
                .document
                .save(
                    &directory.path().join("digital.antitone"),
                    crate::project::Save::New,
                )
                .unwrap();
            let opened =
                crate::editor::document::Document::open(saved.path.as_ref().unwrap()).unwrap();
            assert_eq!(opened.project.roll.exposure, 0.);
            assert!(
                opened.project.sources[0]
                    .grade_snapshot()
                    .unwrap()
                    .layers()
                    .iter()
                    .all(|layer| layer.photo.exposure == 0.)
            );
            assert_eq!(opened.project.sources[0].development, record.development);
        });
    });
}

#[test]
fn negative_viewing_white_and_exposure_update_canvas_and_source_card_without_redevelopment() {
    use crate::raw::SourceWhiteBalance;
    let authored = crate::raw::fixture::bayer_dng();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("negative.dng");
    std::fs::write(
        &path,
        super::super::tests::dng_with_red_as_shot(&std::fs::read(authored.path()).unwrap()),
    )
    .unwrap();
    editor_harness_with_thumbnails(true, |mut h, capture| {
        h.enter(|editor, cx| editor.add_raws(vec![path], cx));
        wait_for(&mut h, |e| {
            !e.interaction_blocked()
                && e.document.visit.working_view.support_ready()
                && e.document
                    .thumbnails
                    .ready_count(&e.document, e.document.target())
                    == 1
        });
        let accepted = h.read(|editor| editor.document.state.project.clone());
        let developed = h.read(|editor| {
            editor
                .document
                .visit
                .canvas
                .ready
                .as_ref()
                .unwrap()
                .developed
                .clone()
        });
        let support = h.read(|editor| {
            editor
                .document
                .visit
                .working_view
                .regional()
                .unwrap()
                .clone()
        });
        h.read(|editor| assert_eq!(editor.source_white_balance, SourceWhiteBalance::Daylight));
        let mut pixels = vec![];
        for (white, ev) in [
            (SourceWhiteBalance::Daylight, 0.),
            (SourceWhiteBalance::AsShot, 0.),
            (SourceWhiteBalance::AsShot, 1.),
        ] {
            h.get_by_role_and_label(egui::accesskit::Role::Button, "Source inspection")
                .click();
            h.step();
            h.step();
            h.get_by_label(match white {
                SourceWhiteBalance::Daylight => "Daylight",
                SourceWhiteBalance::AsShot => "As shot",
            })
            .click();
            h.step();
            enter_numeric(&mut h, "Exposure", &ev.to_string());
            if white == SourceWhiteBalance::AsShot && ev == 1. {
                h.step();
                workflow::wait_settled(&mut h);
                let png = h.render().unwrap();
                save_capture(&png, "negative-inspection-controls-offscreen");
            }
            h.get_by_role_and_label(egui::accesskit::Role::Button, "Source inspection")
                .click();
            h.step();
            workflow::wait_settled(&mut h);
            h.render().unwrap();
            let canvas = h.read(|editor| {
                editor
                    .document
                    .visit
                    .canvas
                    .last_layout
                    .unwrap()
                    .0
                    .image
                    .center()
            });
            // The authored frame is spatially uniform. The card's centre is an
            // interior image point, clear of its caption and selection border.
            let card = h.get_by_label("negative.dng").rect().center();
            let output = capture.borrow();
            let sample = |point: egui::Pos2| {
                output.pixels
                    [point.y.floor() as usize * output.size[0] as usize + point.x.floor() as usize]
            };
            let main = sample(canvas);
            let thumbnail = sample(card);
            for c in 0..3 {
                assert!(
                    (main[c] - thumbnail[c]).abs() < 0.003,
                    "{white:?} EV {ev}: {main:?} != {thumbnail:?}"
                );
            }
            pixels.push(main);
            h.read(|editor| {
                assert_eq!(editor.source_white_balance, white);
                assert_eq!(editor.source_exposure, ev);
                assert_eq!(editor.document.state.project, accepted);
                assert!(Arc::ptr_eq(
                    &editor
                        .document
                        .visit
                        .canvas
                        .ready
                        .as_ref()
                        .unwrap()
                        .developed,
                    &developed
                ));
                assert!(
                    editor
                        .document
                        .visit
                        .working_view
                        .regional()
                        .unwrap()
                        .same_source_as(&support)
                );
            });
        }
        for adjacent in pixels.windows(2) {
            assert!(
                (0..3).any(|c| (adjacent[0][c] - adjacent[1][c]).abs() > 0.01),
                "viewing controls must change rendered colour/brightness"
            );
        }
    });
}

#[test]
fn selecting_and_zooming_a_calibrated_photo_paints_proxy_while_acquisition_is_running() {
    selecting_and_zooming_with_blocked_acquisition(false);
}

#[test]
fn grain_navigation_waits_for_rendered_scopes_while_acquisition_is_running() {
    selecting_and_zooming_with_blocked_acquisition(true);
}

fn selecting_and_zooming_with_blocked_acquisition(film: bool) {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    };

    struct ReleaseOnDrop(mpsc::Sender<()>);
    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }

    editor_harness(|mut harness, _| {
        let first = fixture("roll-0.dng");
        let large = film.then(|| {
            crate::dng_fixture::Fixture {
                side: 1280,
                ..Default::default()
            }
            .write()
        });
        let second = large
            .as_ref()
            .map_or_else(|| fixture("roll-1.dng"), |file| file.path().to_owned());
        harness.enter(|editor, cx| editor.add_raws(vec![first, second.clone()], cx));
        wait_opened_photo(&mut harness);
        if film {
            harness.enter(|editor, cx| {
                editor.apply_input(crate::processing::InputSettings::Digital, cx)
            });
            wait_for(&mut harness, |e| !e.interaction_blocked());
        }
        harness.update(|editor| {
            if film {
                editor.document.project.output.grain = GrainPreset::ThirtyFiveMm.settings();
                editor.scopes.visible = true;
            } else {
                editor.set_dmin(Some(DVec3::splat(0.7)));
                editor.set_dmax(Some(DVec3::splat(0.01)));
            }
            editor.document.source = false;
        });
        wait_for(&mut harness, |e| {
            e.document.visit.working_view.support_ready()
        });
        let previous = harness.read(|editor| editor.document.visit.current_source.unwrap());
        harness.enter(|editor, cx| editor.load(second, cx));
        // Deliver source readiness without painting or starting physical-view acquisition.
        harness.run_until(Duration::from_secs(15), |e| {
            !e.interaction_blocked() && !e.document.visit.source.pending()
        });
        harness.read(|editor| {
            assert_ne!(editor.document.visit.current_source.unwrap(), previous);
            assert!(editor.document.visit.canvas.ready.is_some());
            if film {
                assert_eq!(
                    editor.document.project.input,
                    crate::processing::InputSettings::Digital
                );
                assert!(editor.document.project.output.grain.enabled);
                let ready = editor.document.visit.canvas.ready.as_ref().unwrap();
                let proxy = ready.proxy.image(editor.document.target()).unwrap();
                assert!(
                    proxy.linear.width() < ready.area.width,
                    "fixture must exercise a reduced camera proxy"
                );
            } else {
                assert!(
                    editor.document.project.input.is_negative()
                        && editor.document.project.input.calibration().is_some()
                );
            }
            assert!(
                !editor.document.source && editor.document.visit.working_view.displayed.is_none()
            );
        });

        let (started, starts) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let release = ReleaseOnDrop(release);
        let resume = std::sync::Mutex::new(resume);
        let entered = Arc::new(AtomicBool::new(false));
        let resumed = Arc::new(AtomicBool::new(false));
        let worker_resumed = resumed.clone();
        let ui_thread = std::thread::current().id();
        let ui_submissions = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed_ui = ui_submissions.clone();
        harness.read(|editor| {
            editor.submissions.observe_submissions(Arc::new(move || {
                // Provisional rendering shares the host gate. Park only the background
                // acquisition, not the UI submission whose nonblocking behavior we test.
                if std::thread::current().id() == ui_thread {
                    observed_ui.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                if !entered.swap(true, Ordering::Relaxed) {
                    started.send(()).unwrap();
                    // The timeout only releases a broken implementation for failure/cleanup;
                    // the UI must finish before this running acquisition is released.
                    let _ = resume.lock().unwrap().recv_timeout(Duration::from_secs(10));
                    worker_resumed.store(true, Ordering::Release);
                }
            }));
        });
        harness.step();
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            harness.application.pump();
            if starts.try_recv().is_ok() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "physical acquisition did not reach submission"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        harness.read(|editor| {
            assert!(editor.document.visit.working_view.task.is_some());
            assert!(!editor.document.visit.working_view.support_ready());
        });

        harness.get_by_label("100%").click();
        harness.step();
        harness.render().unwrap();
        assert!(
            !resumed.load(Ordering::Acquire),
            "painting and zoom must finish before the running acquisition resumes"
        );
        assert!(
            ui_submissions.load(Ordering::Relaxed) > 0,
            "provisional rendering must use the host gate while acquisition is parked"
        );
        harness.read(|editor| {
            assert_eq!(editor.document.visit.canvas.viewport.zoom, Some(1.));
            assert!(
                !editor.document.visit.working_view.support_ready()
                    && editor.document.visit.working_view.displayed.is_none()
            );
            let scopes::ScopeInput { view: scope, .. } = editor.scope_request().unwrap();
            let preview = editor
                .document
                .visit
                .canvas
                .preview_rendered
                .as_ref()
                .expect("available proxy preview");
            assert!(
                Arc::ptr_eq(&scope, &preview.2),
                "scopes follow the painted fallback even while acquisition is parked"
            );
            assert!(
                preview
                    .0
                    .same_source(&editor.document.visit.canvas.ready.as_ref().unwrap().proxy),
                "preview consumes the selected photo's actual camera value"
            );
            assert!(
                editor
                    .document
                    .visit
                    .canvas
                    .selection_timing
                    .as_ref()
                    .unwrap()
                    .ready_was_painted()
            );
        });

        drop(release);
        wait_for(&mut harness, |e| {
            e.document.visit.working_view.support_ready()
        });
        harness.render().unwrap();
        harness.read(|editor| {
            assert!(editor.document.visit.canvas.preview_rendered.is_none());
            let displayed = editor
                .document
                .visit
                .working_view
                .displayed
                .as_ref()
                .unwrap();
            let scopes::ScopeInput { view, .. } = editor.scope_request().unwrap();
            assert!(Arc::ptr_eq(&view, displayed));
            if film {
                assert_eq!(
                    view.processing(),
                    editor
                        .positive_processing(
                            editor.document.project.input,
                            editor.document.project.roll,
                            editor.photo_settings()
                        )
                        .unwrap()
                        .clone()
                );
            }
        });
    });
}

#[test]
fn sony_source_inspection_and_positive_share_automatic_development() {
    let order = sony::Order::Little;
    let mut tags = sony::profile_tags(order);
    tags.push((0x7032, 8, order.signed(&[2, 8192, 8192])));
    let file = sony::source(order, tags, false, false);
    editor_harness(|mut h, _| {
        // The authored TIFF has sensor/lens data but no maker-note WB; declare fixed roll gains.
        h.update(|editor| {
            editor.document.state.project.scan = Some(scan_setup(file.path(), [1.; 4]));
        });
        h.enter(|editor, cx| editor.add_raws(vec![file.path().into()], cx));
        wait_for(&mut h, |e| {
            !e.interaction_blocked() && e.document.visit.working_view.support_ready()
        });
        h.read(|editor| assert!(editor.document.source));
        h.render().unwrap();
        h.step();
        let accepted = h.read(|editor| editor.document.state.project.clone());
        let development = h.read(|editor| {
            editor
                .document
                .visit
                .canvas
                .ready
                .as_ref()
                .unwrap()
                .record
                .development
                .clone()
        });
        assert!(
            h.read(|e| e
                .document
                .visit
                .canvas
                .ready
                .as_ref()
                .unwrap()
                .developed
                .precision_floor())
                > crate::processing::CAMERA_INPUT_FLOOR
        );
        let original = h.read(|editor| {
            editor
                .document
                .visit
                .working_view
                .regional()
                .unwrap()
                .clone()
        });
        assert!(original.spec.is_inspection());
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Source inspection")
            .click();
        h.step();
        enter_numeric(&mut h, "Exposure", "0.37");
        h.read(|editor| assert_eq!(editor.source_exposure, 0.37));
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Source inspection")
            .click();
        h.step();
        h.render().unwrap();
        h.read(|editor| {
            assert!(
                editor
                    .document
                    .visit
                    .working_view
                    .regional()
                    .unwrap()
                    .same_source_as(&original)
            )
        });
        h.read(|editor| assert_eq!(editor.document.state.project, accepted));

        h.update(|editor| editor.set_dmin(Some(DVec3::splat(0.7))));
        h.update(|editor| editor.set_dmax(Some(DVec3::splat(0.01))));

        let calibrated = h.read(|editor| editor.document.state.project.clone());
        for source in [false, true] {
            h.get_by_label("View source").click();
            h.step();
            wait_for(&mut h, |e| {
                e.document.visit.working_view.support_ready()
                    && e.document
                        .visit
                        .working_view
                        .displayed
                        .as_ref()
                        .is_some_and(|v| v.spec.is_inspection() == source)
            });
            h.render().unwrap();
            h.read(|editor| assert_eq!(editor.document.source, source));
            h.read(|editor| {
                assert_eq!(
                    editor
                        .document
                        .visit
                        .canvas
                        .ready
                        .as_ref()
                        .unwrap()
                        .record
                        .development,
                    development
                )
            });
            h.read(|editor| assert_eq!(editor.document.state.project, calibrated));
        }
        h.read(|editor| assert_eq!(editor.source_exposure, 0.37));
    });
}

#[test]
fn physical_view_uncalibrated_source_keeps_detail_and_underlay_through_inspection_and_navigation() {
    editor_harness(|mut harness, _| {
        harness.update(|editor| {
            editor
                .document
                .thumbnails
                .set_limits(cache_limits::CacheLimits {
                    cpu_bytes: 0,
                    gpu_bytes: 0,
                })
        });
        harness.enter(|editor, cx| editor.add_raws(vec![fixture("patches.dng")], cx));
        wait_for(&mut harness, |e| {
            !e.interaction_blocked() && e.document.visit.working_view.support_ready()
        });
        harness.read(|editor| assert!(editor.document.source));
        harness.read(|editor| assert!(editor.calibration().is_none()));
        stage(&mut harness, "Rendering");
        harness.update(|editor| editor.document.visit.canvas.viewport.zoom = Some(4.));
        harness.step();
        wait_for(&mut harness, |e| {
            e.document.visit.working_view.support_ready()
        });
        harness.render().unwrap();
        let retained = harness.read(|editor| {
            editor
                .document
                .visit
                .working_view
                .regional()
                .unwrap()
                .clone()
        });
        assert!(retained.spec.is_inspection());
        let mut project = harness.read(|editor| editor.document.state.project.clone());
        // A remembered positive comparison must neither replace source working detail on idle nor
        // force an optional reference past the zero budget while its control is hidden.
        harness.update(|editor| editor.document.visit.working_view.compare = true);
        let hold = crate::background::testing::pause();
        for (index, delta) in [egui::vec2(400., 0.), egui::vec2(-800., 100.)]
            .into_iter()
            .enumerate()
        {
            let layout = harness.read(|editor| editor.active_canvas().last_layout.unwrap().0);
            harness.update(|editor| editor.document.visit.canvas.viewport.pan(layout, 1., delta));
            harness.step();
            harness.step();
            harness
                .get_by_role_and_label(egui::accesskit::Role::Button, "Source inspection")
                .click();
            harness.step();
            enter_numeric(
                &mut harness,
                "Exposure",
                if index == 0 { "0.37" } else { "1.2" },
            );
            harness.read(|editor| {
                assert_eq!(editor.source_exposure, if index == 0 { 0.37 } else { 1.2 })
            });
            harness
                .get_by_role_and_label(egui::accesskit::Role::Button, "Source inspection")
                .click();
            harness.step();
            if index == 1 {
                select_target(&mut harness, OutputTarget::Srgb);
                project.output.tiff = crate::project::TiffTarget::Srgb;
            }
            wait_for(&mut harness, |e| {
                e.document
                    .visit
                    .working_view
                    .displayed
                    .as_ref()
                    .is_some_and(|v| {
                        v.spec.frame() == e.document.visit.working_view.current_frame().unwrap()
                            && Some(&v.processing())
                                == e.document.visit.working_view.processing().as_ref()
                            && v.target == e.document.target()
                    })
            });
            harness.render().unwrap();
            harness.read(|editor| assert!(!editor.document.visit.working_view.support_ready()));
            harness.read(|editor| {
                assert!(
                    editor
                        .document
                        .visit
                        .working_view
                        .regional()
                        .unwrap()
                        .same_source_as(&retained)
                )
            });
            harness.read(|editor| {
                assert!(
                    editor
                        .document
                        .visit
                        .working_view
                        .displayed
                        .as_ref()
                        .unwrap()
                        .spec
                        .is_inspection()
                )
            });
            harness.read(|editor| {
                let scopes::ScopeInput { view: scope, .. } = editor.scope_request().unwrap();
                assert!(scope.spec.is_inspection());
                assert!(Arc::ptr_eq(
                    &scope,
                    &editor.canvas_frame.displayed.as_ref().unwrap().view
                ));
            });
            harness.read(|editor| assert!(!editor.document.visit.working_view.reference_ready()));
            harness.read(|editor| assert!(!editor.document.visit.canvas.original.refining()));
            harness.read(|editor| {
                assert_eq!(
                    editor.document.state.project, project,
                    "inspection EV and navigation do not alter the document"
                )
            });
            harness.read(|editor| assert!(
            !Arc::ptr_eq(
                editor.document.visit.working_view.displayed.as_ref().unwrap(),
                editor.document.visit.working_view.regional().unwrap()
            ),
            "the physical-frame underlay composite, not only a transformed regional patch, is painted"
        ));
            let held = harness.read(|editor| {
                editor
                    .document
                    .visit
                    .working_view
                    .displayed
                    .as_ref()
                    .unwrap()
                    .clone()
            });
            for _ in 0..5 {
                harness.step();
            }
            harness.render().unwrap();
            harness.read(|editor| {
                assert!(
                    Arc::ptr_eq(
                        &held,
                        editor
                            .document
                            .visit
                            .working_view
                            .displayed
                            .as_ref()
                            .unwrap()
                    ),
                    "idle does not replace source detail"
                )
            });
            harness.read(|editor| assert!(editor.document.visit.working_view.task.is_some()));
        }
        drop(hold);
        wait_for(&mut harness, |e| {
            e.document.visit.working_view.support_ready()
        });
        harness.render().unwrap();
        harness.read(|editor| {
            assert!(
                !editor
                    .document
                    .visit
                    .working_view
                    .regional()
                    .unwrap()
                    .same_source_as(&retained)
            )
        });
        harness.read(|editor| {
            assert!(Arc::ptr_eq(
                editor
                    .document
                    .visit
                    .working_view
                    .displayed
                    .as_ref()
                    .unwrap(),
                editor.document.visit.working_view.regional().unwrap()
            ))
        });
    });
}
