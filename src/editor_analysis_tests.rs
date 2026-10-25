use super::*;
use crate::project::Development;
use crate::project::SourceDevelopment;

#[test]
fn mixed_developed_floors_use_one_conservative_analysis_bound_and_recheck_sample_counts() {
    let native = crate::processing::CAMERA_INPUT_FLOOR;
    let frame = |floor| FrameMeasurements {
        floor,
        film: crate::scan::AnalysisSamples {
            pixels: vec![DVec3::splat(0.8); 2048],
            ..Default::default()
        },
        picture: crate::scan::AnalysisSamples {
            pixels: (0..1024)
                .map(|i| DVec3::splat(0.005 * 100f64.powf(i as f64 / 1023.)))
                .collect(),
            ..Default::default()
        },
    };
    let mut owned = vec![frame(native), frame(native * 2.)];
    owned[0]
        .picture
        .pixels
        .extend([DVec3::splat(native * 1.5), DVec3::splat(native * 2.)]);
    let floor = restrict_to_common_floor(&mut owned).unwrap();
    assert_eq!(floor, native * 2.);
    assert_eq!(owned[0].picture.pixels.len(), 1024);
    assert_eq!(owned[0].picture.rejected_floor, 2);
    assert_eq!(owned[1].picture.rejected_floor, 0);
    let frames: Vec<_> = owned
        .iter()
        .map(|f| FrameSamples {
            film: &f.film.pixels,
            picture: &f.picture.pixels,
        })
        .collect();
    let proposal = roll_analysis::estimate(
        &frames,
        floor,
        glam::DMat3::IDENTITY,
        crate::processing::NegativeInput::from(Calibration {
            dmin: DVec3::splat(0.8),
            dmax: DVec3::splat(0.02),
            input_floor: floor,
        }),
        || false,
    )
    .unwrap()
    .unwrap();
    assert_eq!(proposal.photos.len(), 2);
    require_statistics(4096, &proposal.frames).unwrap();
    let mut sparse = vec![frame(native), frame(native * 2.)];
    sparse[0].picture.pixels = vec![DVec3::splat(native * 1.5); 1024];
    assert!(restrict_to_common_floor(&mut sparse).is_err());
}

#[test]
fn authored_roll_runs_the_editor_worker_and_preserves_failure_cancel_and_save_open_state() {
    pollster::block_on(async {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw");
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance
            .request_adapter(&Default::default())
            .await
            .expect("authored worker test requires Metal");
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let mut editor = Editor::new(&device, &queue);
        // Thumbnail scheduling is separate; this worker test must not touch the user's disk cache.
        editor.thumbnails = Thumbnails::default();
        editor = crate::editor::tests::complete_document_operation(
            editor,
            Duration::from_secs(60),
            |editor, cx| {
                editor.add_raws(vec![root.join("roll-0.dng"), root.join("roll-1.dng")], cx)
            },
        );
        assert!(editor.document.visit.ready.is_some(), "{}", editor.status);
        assert_eq!(editor.document.project.order.len(), 2);
        let selected = editor.current_source;
        editor.set_dmin(Some(DVec3::splat(0.8)));
        editor.set_dmax(Some(DVec3::splat(0.02)));
        editor.set_floor(crate::processing::CAMERA_INPUT_FLOOR);
        editor.document.project.analysis_area = Some([96, 96, 320, 320]);
        editor.set_gamma(0.7);
        editor.roll_orientation.turn_clockwise();
        editor.photo_orientation.flips.horizontal = true;
        editor.crop = Some(crate::crop::Crop {
            center: [256., 256.],
            size: [288, 288],
            angle: 0.05,
        });
        editor.sync_document();
        let before = editor.document.project.clone();
        editor = click_analyze(editor, Duration::from_secs(60));
        assert!(editor.status.starts_with("Analyzed 2"), "{}", editor.status);
        assert_eq!(editor.current_source, selected);
        assert_eq!(editor.gamma(), 0.7);
        assert_eq!(editor.response(), before.input.response());
        assert_eq!(editor.document.project.orientation, before.orientation);
        for (source, prior) in editor.document.project.sources.iter().zip(&before.sources) {
            assert_eq!(source.orientation, prior.orientation);
            assert_eq!(source.crop, prior.crop);
            assert_eq!(source.photo.wb, Balance::default());
        }
        let expected = 3000. / 4031.;
        assert!(
            editor
                .document
                .project
                .dmin()
                .unwrap()
                .iter()
                .all(|v| (v - expected).abs() <= 2. / 65535.)
        );
        let ids = &editor.document.project.order;
        assert!(
            editor
                .document
                .project
                .source(ids[0])
                .unwrap()
                .photo
                .exposure
                > editor
                    .document
                    .project
                    .source(ids[1])
                    .unwrap()
                    .photo
                    .exposure
        );
        let accepted = editor.document.project.clone();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("authored-roll.antitone.json");
        editor = crate::editor::tests::complete_document_operation(
            editor,
            Duration::from_secs(60),
            |editor, cx| editor.save_project(path.clone(), crate::project::Save::New, None, cx),
        );
        assert!(!editor.document.dirty(), "{}", editor.status);
        editor = crate::editor::tests::complete_document_operation(
            editor,
            Duration::from_secs(60),
            |editor, cx| editor.start_document(Preparation::Open(path), cx),
        );
        assert_eq!(editor.document.project, accepted);
        editor.document.project.analysis_area = Some([96, 96, 8, 8]);
        let sparse = editor.document.project.clone();
        editor = wait_with_limit(editor, Duration::from_secs(60), |editor, cx| {
            editor.begin_analysis(cx)
        });
        assert!(
            editor.status.contains("picture samples"),
            "{}",
            editor.status
        );
        assert_eq!(editor.document.project, sparse);
        editor.document.project.analysis_area = accepted.analysis_area;
        let unvisited = editor.document.project.order[1];
        let camera = editor
            .document
            .project
            .source(unvisited)
            .unwrap()
            .development
            .camera_model
            .clone();
        editor
            .document
            .project
            .source_mut(unvisited)
            .unwrap()
            .development
            .camera_model = "incompatible-camera".into();
        let incompatible = editor.document.project.clone();
        editor = wait_with_limit(editor, Duration::from_secs(60), |e, cx| {
            e.begin_analysis(cx);
            assert!(e.busy && e.document.analysis_pending.is_some());
        });
        assert!(editor.status.contains("failed"), "{}", editor.status);
        assert_eq!(editor.document.project, incompatible);
        editor
            .document
            .project
            .source_mut(unvisited)
            .unwrap()
            .development
            .camera_model = camera;
        let before_cancel = editor.document.project.clone();
        editor = wait_with_limit(editor, Duration::from_secs(60), |e, cx| {
            e.begin_analysis(cx);
            assert!(e.document.analysis_pending.is_some() && e.busy);
            e.cancel_analysis();
        });
        assert!(editor.document.analysis_pending.is_none());
        assert!(!editor.busy);
        assert_eq!(editor.status, "Analysis cancelled");
        assert_eq!(editor.document.project, before_cancel);
        assert!(editor.document.visit.ready.is_some());
    });
}

fn state() -> Editor {
    let mut editor = Editor::state();
    let id = editor
        .document
        .register_with_development(
            Source {
                id: SourceId(0),
                path: "scene.ARW".into(),
                development: SourceDevelopment {
                    decode: Development::new("test-backend".into(), [1.; 4]),
                    lens_correction: None,
                    dng_development: None,
                    dimensions: [824, 624],
                    active_area: [12, 12, 800, 600],
                    camera_model: "test-camera".into(),
                    camera_to_ap1: glam::DMat3::IDENTITY.to_cols_array_2d(),
                    input_floor: crate::processing::CAMERA_INPUT_FLOOR,
                },
                reference_only: false,
                photo: PhotoSettings::default(),
                local_adjustments: None,
                repairs: Default::default(),
                orientation: Default::default(),
                crop: None,
            },
            Development::new(crate::raw::version(), [2., 1., 1.7, 1.]),
        )
        .unwrap();
    editor.id = 5;
    editor.next_request = 5;
    editor.current_source = Some(id);
    editor.set_floor(crate::processing::CAMERA_INPUT_FLOOR);
    editor.set_gamma(0.9);
    editor.roll.exposure = 0.;
    editor.photo.exposure = 1.23456789;
    editor.photo.wb = Balance {
        warmth: 0.3,
        tint: -0.2,
    };
    editor.photo.contrast = 0.8;
    editor.photo.pivot = 0.3;
    editor.photo.contrast_colour_mix = 0.4;
    editor.photo.saturation = 1.2;
    editor.photo.saturation_density = 0.6;
    editor.roll.wb = Balance {
        warmth: 0.2,
        tint: -0.1,
    };
    editor.roll_orientation.turn_clockwise();
    editor.photo_orientation.flips.horizontal = true;
    editor.crop = Some(crate::crop::Crop {
        center: [412., 312.],
        size: [600, 400],
        angle: 0.1,
    });
    editor.sync_document();
    editor
}

fn pending(editor: &mut Editor) -> u64 {
    let id = editor.request_id();
    let revision = editor
        .document
        .analysis_generation
        .fetch_add(1, Ordering::Relaxed)
        + 1;
    editor.document.analysis_pending = Some(analysis_workflow::Pending::new(
        id,
        Request {
            loaded: editor.id,
            revision,
            document: editor.document.capture(),
            selected: editor.current_source.unwrap(),
            film: Some(rect([12, 12, 800, 600])),
            local_support: false,
            picture: Some(rect([52, 42, 720, 540])),
            areas: None,
        },
    ));
    editor.tasks.begin(Task::Analyze(id));
    editor.tasks.report(
        TaskRequest::Analyze(id),
        TaskProgress::Counted {
            completed: 0,
            total: 1,
        },
    );
    editor.busy = true;
    id
}

fn outcome(editor: &Editor) -> Outcome {
    let mut document = editor
        .document
        .analysis_pending
        .as_ref()
        .unwrap()
        .request
        .document
        .clone();
    document.project.set_dmin(Some([0.8, 0.4, 0.2]));
    document.project.set_dmax(Some([0.04, 0.02, 0.01]));
    document.project.set_balance(Balance {
        warmth: 0.1,
        tint: -0.05,
    });
    document.project.roll.exposure = 2.1;
    document.project.roll.wb = Balance {
        warmth: -0.2,
        tint: 0.1,
    };
    for (index, source) in document.project.sources.iter_mut().enumerate() {
        source.photo = PhotoSettings {
            exposure: if index == 0 { 0.4 } else { -0.4 },
            wb: Balance::default(),
            ..PhotoSettings::default()
        };
    }
    document.project.analysis_area = Some([52, 42, 720, 540]);
    let frames = document.project.order.len();
    Outcome {
        document,
        frames,
        support: rect([12, 12, 800, 600]),
    }
}

#[test]
fn one_complete_reply_reinitializes_all_grades_including_authored_zero_preserving_geometry() {
    let mut editor = state();
    let mut second = editor.document.project.sources[0].clone();
    second.id = SourceId(0);
    second.path = "second.ARW".into();
    second.photo.exposure = 0.;
    editor.document.project.add_source(second).unwrap();
    let before = editor.document.project.clone();
    editor.viewport.zoom = Some(1.7);
    let area = rect([12, 12, 800, 600]);
    let composition = Composition::new(area, Default::default(), Default::default()).unwrap();
    let frame = crate::frame_raster::FrameRaster::full(composition).unwrap();
    let input = ProcessingInput::from(ProcessingParameters::source());
    editor
        .working
        .observe(editor.id, frame, input.signature(), editor.target);
    assert!(editor.working.revision().is_some());
    let view = ViewRequest {
        area,
        output: [32, 24],
        frame: None,
    };
    editor.original.observe(
        OriginalPurpose::Reference,
        (input.signature(), editor.target, Some(view), composition),
    );
    let original_revision = editor.original.revision();
    editor.original.refining = Some((original_revision, view));
    let id = pending(&mut editor);
    editor.analysis_progress(id, 1, 1);
    assert_eq!(editor.document.project, before);
    assert!(editor.busy);
    let result = outcome(&editor);
    let expected = result.document.project.clone();
    editor.analysis_result(
        id,
        Ok(Some(Box::new(result))),
        &crate::application::UiContext::render_only(),
    );
    assert!(!editor.busy && editor.document.analysis_pending.is_none());
    assert_eq!(editor.dmin().unwrap(), DVec3::new(0.8, 0.4, 0.2));
    assert_eq!(editor.dmax().unwrap(), DVec3::new(0.04, 0.02, 0.01));
    assert_eq!(editor.roll.exposure, 2.1);
    assert_eq!(editor.gamma(), before.input.gamma());
    assert_eq!(editor.response(), before.input.response());
    assert_eq!(editor.roll.wb, expected.roll.wb);
    assert_eq!(editor.photo, expected.sources[0].photo);
    assert_ne!(editor.photo, before.sources[0].photo);
    assert_eq!(editor.document.project.sources[1].photo.exposure, -0.4);
    for (mut after, prior) in editor
        .document
        .project
        .sources
        .clone()
        .into_iter()
        .zip(&before.sources)
    {
        assert_eq!(
            after.photo,
            PhotoSettings {
                exposure: after.photo.exposure,
                ..PhotoSettings::default()
            }
        );
        after.photo = prior.photo;
        assert_eq!(&after, prior);
    }
    assert_eq!(editor.document.project.development(), before.development());
    assert_eq!(
        editor.document.project.camera_basis(),
        before.camera_basis()
    );
    assert_eq!(editor.viewport.zoom, Some(1.7));
    assert!(!editor.source);
    assert!(editor.working.revision().is_none());
    assert!(editor.working.displayed.is_none());
    assert!(editor.original.signature.is_none() && editor.original.settled.is_none());
    assert!(editor.original.revision() > original_revision);
    assert!(editor.original.refining.is_none());
    editor.original_result(original_revision, Ok(None));
    assert!(editor.original.refining.is_none() && editor.original.settled.is_none());
    assert!(editor.document.dirty());
    editor.roll.exposure = 0.;
    editor.photo.exposure = 0.;
    editor.photo.wb = Balance {
        warmth: 0.25,
        tint: -0.15,
    };
    let manual_photo = editor.photo;
    editor.sync_document();
    assert_eq!(editor.roll.exposure, 0.); // Manual zero is not an auto-placement trigger.
    assert_eq!(editor.photo, manual_photo);
}

#[test]
fn failure_cancel_and_invalid_gpu_combination_preserve_all_processing_state() {
    for mode in 0..3 {
        let mut editor = state();
        let before = editor.document.project.clone();
        let id = pending(&mut editor);
        let mut result = outcome(&editor);
        if mode == 1 {
            editor.cancel_analysis();
        }
        if mode == 2 {
            result.document.project.set_gamma(0.1);
            result.document.project.set_balance(Balance {
                warmth: 0.8,
                tint: 0.8,
            });
            result
                .document
                .project
                .set_response(crate::processing::Response::Film {
                    toe: 1.,
                    shoulder: 0.02,
                });
        }
        editor.analysis_result(
            id,
            if mode == 0 {
                Err("sampling failed".into())
            } else {
                Ok(Some(Box::new(result)))
            },
            &crate::application::UiContext::render_only(),
        );
        assert_eq!(editor.document.project, before);
        assert_eq!(editor.roll, before.roll);
        assert_eq!(editor.photo, before.sources[0].photo);
        assert!(!editor.busy);
    }
}

#[test]
fn stale_request_generation_source_document_and_controls_cannot_install() {
    for mode in 0..5 {
        let mut editor = state();
        let id = pending(&mut editor);
        let result = outcome(&editor);
        match mode {
            0 => {
                editor.cancel_analysis();
            }
            1 => {
                editor.id += 1;
            }
            2 => {
                editor.document.path = Some("other.antitone.json".into());
            }
            3 => {
                editor.set_gamma(1.);
                editor.sync_document();
            }
            _ => {
                editor.document.project.sources[0].path = "replacement.ARW".into();
            }
        }
        let before = editor.document.project.clone();
        editor.analysis_result(
            id + 1,
            Ok(None),
            &crate::application::UiContext::render_only(),
        );
        assert!(editor.busy);
        editor.analysis_result(
            id,
            Ok(Some(Box::new(result))),
            &crate::application::UiContext::render_only(),
        );
        assert_eq!(editor.document.project, before);
    }
}

#[test]
fn area_accept_cancel_and_invalid_edits_do_not_change_calibration_or_exposure() {
    let mut editor = state();
    let original = editor.document.project.clone();
    editor.begin_pick(
        Pick::Analysis,
        &crate::application::UiContext::render_only(),
    );
    editor.region = Some(Coverage::rectangle(rect([52, 42, 720, 540])));
    editor.finish_pick(&crate::application::UiContext::render_only());
    assert_eq!(editor.document.project, original);
    editor.begin_pick(
        Pick::Analysis,
        &crate::application::UiContext::render_only(),
    );
    editor.region = Some(Coverage::rectangle(rect([0, 0, 900, 700])));
    editor.apply_analysis_area(&crate::application::UiContext::render_only());
    assert_eq!(editor.document.project, original);
    assert_eq!(editor.pick, Some(Pick::Analysis));
    editor.region = Some(Coverage::rectangle(rect([52, 42, 720, 540])));
    editor.apply_analysis_area(&crate::application::UiContext::render_only());
    assert!(editor.pick.is_none());
    assert_eq!(
        editor.document.project.analysis_area,
        Some([52, 42, 720, 540])
    );
    assert_eq!(editor.roll, original.roll);
    assert_eq!(editor.document.project.dmin(), original.dmin());
    assert_eq!(editor.document.project.dmax(), original.dmax());
}

#[test]
fn failed_or_cancelled_analysis_preserves_original_comparison_or_restarts_requested_work() {
    for (settled, outstanding) in [(false, true), (true, true), (true, false)] {
        for cancel in [false, true] {
            let mut editor = super::super::tests::authored_fixture(&["patches.dng"]);
            editor.input = super::super::tests::fixture().input;
            editor.source = false;
            editor.working.compare = true;
            editor.sync_document();
            crate::application::test_support::with_application(editor, |app| {
                let ctx = egui::Context::default();
                app.enter(|editor, cx| {
                    for _ in 0..4 {
                        let mut output = ctx.run_ui(
                            egui::RawInput {
                                screen_rect: Some(egui::Rect::from_min_size(
                                    egui::Pos2::ZERO,
                                    egui::vec2(1100., 800.),
                                )),
                                ..Default::default()
                            },
                            |ui| editor.draw_ui(ui, cx),
                        );
                        output.textures_delta.clear();
                    }
                });
                app.run_until(Duration::from_secs(15), |editor| {
                    editor.working.support_ready()
                });
                app.enter(|editor, cx| {
                    let frame = |editor: &mut Editor| {
                        let mut output = ctx.run_ui(
                            egui::RawInput {
                                screen_rect: Some(egui::Rect::from_min_size(
                                    egui::Pos2::ZERO,
                                    egui::vec2(1100., 800.),
                                )),
                                ..Default::default()
                            },
                            |ui| editor.draw_ui(ui, cx),
                        );
                        output.textures_delta.clear();
                    };
                    frame(editor);
                    editor.original.last_edit = std::time::Instant::now() - Duration::from_secs(1);
                    frame(editor);
                    let revision = editor.original.revision();
                    let signature = editor.original.signature.clone();
                    let view = signature.as_ref().unwrap().2.unwrap();
                    let image = editor
                        .document
                        .visit
                        .ready
                        .as_ref()
                        .unwrap()
                        .proxy
                        .image(OutputTarget::Srgb)
                        .unwrap();
                    assert!(editor.original.refining.is_some());
                    if !outstanding {
                        editor.original_result(revision, Ok(Some(image.clone())));
                    } else if settled {
                        editor.original.settled = Some((revision, view, image.clone()));
                    }
                    let before = editor.document.project.clone();
                    editor.begin_analysis(cx);
                    assert_eq!(editor.original.signature, signature);
                    assert_eq!(editor.original.settled.is_some(), settled);
                    if outstanding {
                        // A late completion from cancelled work cannot replace the accepted image.
                        editor.original_result(revision, Ok(Some(image.clone())));
                    } else {
                        assert_eq!(editor.original.revision(), revision);
                    }
                    assert_eq!(editor.original.settled.is_some(), settled);
                    let pending = editor.document.analysis_pending.as_ref().unwrap().id;
                    if cancel {
                        editor.cancel_analysis();
                    }
                    editor.analysis_result(
                        pending,
                        if cancel {
                            Ok(None)
                        } else {
                            Err("sampling failed".into())
                        },
                        cx,
                    );
                    assert_eq!(editor.document.project, before);
                    frame(editor);
                    if settled {
                        let (current_revision, current_view, retained) =
                            editor.original.settled.as_ref().unwrap();
                        assert_eq!(*current_revision, editor.original.revision());
                        assert_eq!(*current_view, view);
                        assert!(Arc::ptr_eq(retained, &image));
                        assert!(editor.original.refining.is_none());
                        assert!(editor.document.visit.grade.task.is_none());
                    } else {
                        assert!(editor.original.refining.is_some());
                        assert!(editor.document.visit.grade.task.is_some());
                    }
                });
            });
        }
    }
}

#[test]
fn statistics_edit_bypasses_composition_and_crop_drafts_cannot_start_analysis() {
    let editor = super::super::tests::fixture();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.roll_orientation.turn_clockwise();
            editor.photo_orientation.flips.horizontal = true;
            editor.crop = Some(crate::crop::Crop {
                center: [444., 300.],
                size: [360, 240],
                angle: 0.1,
            });
            super::super::tests::add_ready_fixture(editor);
            editor.source = false;
            editor.sync_document();
            let before = editor.document.project.clone();
            editor.viewport.zoom = Some(1.7);
            let view = editor.viewport;
            editor.begin_pick(Pick::Analysis, cx);
            let area = editor.document.visit.ready.as_ref().unwrap().area;
            let ctx = egui::Context::default();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100., 800.),
                    )),
                    ..Default::default()
                },
                |ui| crate::editor::tests::paint_controller(editor, ui),
            );
            output.textures_delta.clear();
            assert!(editor.source);
            assert_eq!(
                editor.last_layout.unwrap().0.composition,
                Composition::new(
                    area,
                    RollOrientation::default(),
                    PhotoOrientation::default()
                )
                .unwrap()
            );
            editor.region = Some(Coverage::rectangle(rect([52, 42, 720, 480])));
            editor.apply_analysis_area(cx);
            assert!(editor.pick.is_none());
            assert_eq!(
                editor.document.project.analysis_area,
                Some([52, 42, 720, 480])
            );
            assert_eq!(editor.document.project.sources, before.sources);
            assert_eq!(editor.document.project.orientation, before.orientation);
            assert_eq!(editor.roll, before.roll);
            assert_eq!(editor.viewport, view);
            assert!(!editor.source);
            editor.begin_crop();
            assert!(editor.document.visit.crop_edit.is_some());
            editor.begin_analysis(&UiContext::render_only());
            assert!(editor.document.analysis_pending.is_none());
            assert!(editor.document.analysis_pending.is_none());
            editor.finish_crop(false);
        });
    });
}

#[test]
fn missing_local_support_never_becomes_whole_camera_picture_statistics() {
    let editor = state();
    assert_eq!(editor.analysis_support(), None);
    assert_eq!(editor.analysis_picture(), None);
    assert!(picture_area(None, None).is_err());
    let support = rect([1372, 46, 5950, 5576]);
    assert_eq!(picture_area(None, Some(support)).unwrap(), suggest(support));
    let manual = rect([1700, 400, 5200, 4900]);
    assert_eq!(picture_area(Some(manual), None).unwrap(), manual);
    assert_eq!(picture_area(Some(manual), Some(support)).unwrap(), manual);
}

#[test]
fn sparse_eligibility_is_rejected_before_any_grade_installation() {
    let mut editor = state();
    let before = editor.document.project.clone();
    let id = pending(&mut editor);
    let samples = crate::scan::AnalysisSamples {
        pixels: vec![DVec3::splat(0.1)],
        attempted: 8192,
        rejected_capture: 8191,
        ..Default::default()
    };
    let error = require_picture(&samples).unwrap_err();
    editor.analysis_result(
        id,
        Err(error.to_string()),
        &crate::application::UiContext::render_only(),
    );
    assert_eq!(editor.document.project, before);
    let enough = roll_analysis::FrameStatistics {
        film_samples: 0,
        picture_samples: 512,
        balance_eligible: 256,
        balance_samples: 256,
        exposure_samples: 512,
    };
    assert!(require_statistics(1, std::slice::from_ref(&enough)).is_err());
    assert!(
        require_statistics(
            2048,
            &[roll_analysis::FrameStatistics {
                balance_samples: 1,
                ..enough.clone()
            }]
        )
        .is_err()
    );
    assert!(
        require_statistics(
            2048,
            &[roll_analysis::FrameStatistics {
                exposure_samples: 1,
                ..enough.clone()
            }]
        )
        .is_err()
    );
    assert!(require_statistics(2048, &[enough]).is_ok()); // Film adequacy is pooled, never per-frame.
}

#[test]
#[ignore = "read-only blue-cast feedback roll/Metal; run explicitly in release"]
fn real_ambiguous_first_frame_uses_roll_opening_instead_of_holder() {
    pollster::block_on(async {
        let root = std::env::var_os("ANTITONE_SAMPLES")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("samples"));
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let mut editor = Editor::new(&device, &queue);
        let mut paths: Vec<_> = [4321, 4322, 4323]
            .iter()
            .map(|n| root.join(format!("DSC0{n}.ARW")))
            .collect();
        paths.extend((4312..=4320).map(|n| root.join(format!("DSC0{n}.ARW"))));
        editor = crate::editor::tests::complete_document_operation(
            editor,
            Duration::from_secs(60),
            |editor, cx| editor.add_raws(paths, cx),
        );
        assert!(editor.document.visit.ready.is_some(), "{}", editor.status);
        assert_eq!(
            editor
                .document
                .visit
                .ready
                .as_ref()
                .unwrap()
                .analysis_support,
            None
        );
        assert_eq!(editor.analysis_picture(), None);
        let before = editor.document.project.clone();
        editor = click_analyze(editor, Duration::from_secs(180));
        assert!(
            editor.status.starts_with("Analyzed 12"),
            "{}",
            editor.status
        );
        assert_eq!(editor.gamma(), before.input.gamma());
        assert_eq!(editor.response(), before.input.response());
        for (mut after, prior) in editor
            .document
            .project
            .sources
            .clone()
            .into_iter()
            .zip(&before.sources)
        {
            assert_eq!(after.photo.wb, Balance::default());
            after.photo = prior.photo;
            assert_eq!(&after, prior);
        }
        let [x, y, w, h] = editor.document.project.analysis_area.unwrap();
        assert!(x > 1600 && y > 300 && w < 5500 && h < 5200);
        let dmax = editor.dmax().unwrap();
        assert!(
            dmax.x > 0.01 && dmax.y > 0.003 && dmax.z > 0.001,
            "holder tails returned: {dmax:?}"
        );
        assert!(
            editor.roll.exposure < 3.,
            "holder-driven placement returned"
        );
        println!(
            "Feedback roll: shared opening recovered from later frame, area={:?}, Dmin={:?} Dmax={dmax:?} balance={:?} EV={}",
            editor.document.project.analysis_area,
            editor.dmin(),
            editor.balance(),
            editor.roll.exposure
        );
    });
}

fn ready_state() -> Editor {
    let mut editor = super::super::tests::fixture();
    super::super::tests::add_ready_fixture(&mut editor);
    editor
}

fn click_label(
    editor: &mut impl crate::editor::tests::UiFixture,
    ctx: &egui::Context,
    label: &str,
) {
    super::super::tests::app_frame(editor, ctx, vec![]);
    let labels = super::super::tests::app_frame(editor, ctx, vec![]);
    let point = labels
        .iter()
        .rfind(|(name, _)| name == label)
        .unwrap_or_else(|| panic!("{label} is visible: {labels:?}"))
        .1
        .center();
    for pressed in [true, false] {
        super::super::tests::app_frame(
            editor,
            ctx,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
}

fn click_analyze(editor: Editor, limit: Duration) -> Editor {
    let owner = std::cell::RefCell::new(Some(editor));
    {
        let mut app = crate::application::test_support::TestApplication::new(&owner, || {});
        let ctx = egui::Context::default();
        click_label(&mut app, &ctx, "Analyze");
        app.run_until(Duration::from_secs(60), |e| {
            e.document.spatial_preparing.is_none() && e.document.analysis_preparing.is_none()
        });
        app.read(|e| assert!(e.document.analysis_review.is_some(), "{}", e.status));
        click_label(&mut app, &ctx, "Analyze roll");
        assert!(app.read(|e| e.document.analysis_pending.is_some()));
        app.run_until(limit, |e| {
            e.document.analysis_pending.is_none() && !e.document.visit.source.pending() && !e.busy
        });
        app.close_execution();
    }
    owner.into_inner().unwrap()
}

#[path = "editor_local_analysis_tests.rs"]
mod locals;
#[path = "editor_analysis_review_tests.rs"]
mod review_tests;

#[test]
fn photo_numeric_focus_commit_precedes_analysis_area_preparation_snapshot() {
    let mut editor = ready_state();
    editor.tab = 1;
    for section in [
        "photo-composition",
        "photo-white-balance",
        "photo-contrast",
        "photo-saturation",
    ] {
        editor.preferences.sections.insert(section.into(), false);
    }
    let ctx = egui::Context::default();
    crate::application::test_support::with_application(editor, |app| {
        for _ in 0..4 {
            super::super::tests::app_frame(app, &ctx, vec![]);
        }
        let labels = super::super::tests::app_frame(app, &ctx, vec![]);
        let exposure = labels
            .iter()
            .find(|(label, _)| label == "Exposure")
            .unwrap()
            .1;
        let point = labels
            .iter()
            .filter(|(label, rect)| {
                label.contains("0.00")
                    && rect.min.y >= exposure.min.y
                    && rect.min.x >= exposure.min.x
            })
            .min_by(|(_, a), (_, b)| a.min.y.total_cmp(&b.min.y))
            .unwrap()
            .1
            .center();
        for pressed in [true, false] {
            super::super::tests::app_frame(
                app,
                &ctx,
                vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert!(
            ctx.text_edit_focused(),
            "numeric point {point:?}, labels {labels:?}"
        );
        super::super::tests::app_frame(app, &ctx, vec![egui::Event::Text("0.7654321".into())]);
        assert_eq!(app.read(|e| e.photo.exposure), 0.);
        click_label(app, &ctx, "Roll");
        app.read(|e| {
            assert_eq!(
                e.photo.exposure, 0.7654321,
                "tab={} status={}",
                e.tab, e.status
            )
        });
        click_label(app, &ctx, "Analyze");
        app.read(|e| {
            let request = e
                .document
                .analysis_preparing
                .as_ref()
                .map(|pending| &pending.request)
                .or_else(|| {
                    e.document
                        .spatial_preparing
                        .as_ref()
                        .and_then(spatial::Pending::analysis_request)
                })
                .expect("captured Analyze request");
            let snapshot = &request.document;
            assert_eq!(
                snapshot
                    .project
                    .source(e.current_source.unwrap())
                    .unwrap()
                    .photo
                    .exposure,
                0.7654321
            );
            assert!(e.document.analysis_pending.is_none());
        });
    });
}

#[test]
fn escape_cancels_preparation_without_invalidating_original_comparison_or_grade() {
    let editor = ready_state();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.photo.contrast = 1.2;
            editor.source = false;
            editor.working.compare = true;
            let ctx = egui::Context::default();
            for _ in 0..4 {
                super::super::tests::app_frame(editor, &ctx, vec![]);
            }
            let signature = editor.original.signature.clone();
            let revision = editor.original.revision();
            let view = signature.as_ref().unwrap().2.unwrap();
            let image = editor
                .document
                .visit
                .ready
                .as_ref()
                .unwrap()
                .proxy
                .image(OutputTarget::Srgb)
                .unwrap();
            editor.original.settled = Some((revision, view, image.clone()));
            editor.request_analysis(cx);
            let before = editor.document.project.clone();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    ..Default::default()
                },
                |ui| editor.frame_shortcuts(ui, cx, &mut None, false, true),
            );
            output.textures_delta.clear();
            assert!(editor.document.analysis_pending.is_none());
            assert!(editor.document.spatial_preparing.is_none());
            assert!(editor.document.analysis_preparing.is_none());
            assert!(editor.tasks.view().is_none());
            assert_eq!(editor.document.project, before);
            assert_eq!(editor.original.signature, signature);
            assert_eq!(editor.original.revision(), revision);
            assert!(Arc::ptr_eq(
                &editor.original.settled.as_ref().unwrap().2,
                &image
            ));
        });
    });
}

fn wait_with_limit(
    editor: Editor,
    limit: Duration,
    start: impl FnOnce(&mut Editor, &UiContext<'_, Editor>),
) -> Editor {
    crate::editor::tests::complete_analysis_operation(editor, limit, start)
}
