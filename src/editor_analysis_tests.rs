use super::*;
use crate::project::SourceDevelopment;

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
        let mut editor = Editor::new(&device, &queue, None);
        // Thumbnail scheduling is separate; this worker test must not touch the user's disk cache.
        editor.thumbnails = Thumbnails::default();
        editor.add_raws(vec![root.join("roll-0.dng"), root.join("roll-1.dng")]);
        wait_with_limit(&mut editor, Duration::from_secs(60));
        assert!(editor.ready.is_some(), "{}", editor.status);
        assert_eq!(editor.document.project.order.len(), 2);
        let selected = editor.current_source;
        editor.document.project.analysis_area = Some([96, 96, 320, 320]);
        editor.roll.gamma = 0.7;
        editor.roll_orientation.turn_clockwise();
        editor.photo_orientation.flips.horizontal = true;
        editor.crop = Some(crate::crop::Crop {
            center: [256., 256.],
            size: [288, 288],
            angle: 0.05,
        });
        editor.sync_document();
        let before = editor.document.project.clone();
        click_analyze(&mut editor);
        assert!(editor.analysis_pending.is_some());
        wait_with_limit(&mut editor, Duration::from_secs(60));
        assert!(editor.status.starts_with("Analyzed 2"), "{}", editor.status);
        assert_eq!(editor.current_source, selected);
        assert_eq!(editor.roll.gamma, 0.7);
        assert_eq!(editor.roll.response, before.roll.response);
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
                .dmin
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
        editor.queue_save(path.clone(), crate::project::Save::New, None);
        wait_with_limit(&mut editor, Duration::from_secs(60));
        assert!(!editor.document.dirty(), "{}", editor.status);
        editor.start_document(DocumentWork::Open(path));
        wait_with_limit(&mut editor, Duration::from_secs(60));
        assert_eq!(editor.document.project, accepted);
        editor.document.project.analysis_area = Some([96, 96, 8, 8]);
        let sparse = editor.document.project.clone();
        editor.begin_analysis();
        wait_with_limit(&mut editor, Duration::from_secs(60));
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
        editor.begin_analysis();
        assert!(editor.busy);
        wait_with_limit(&mut editor, Duration::from_secs(60));
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
        editor.begin_analysis();
        assert!(editor.analysis_pending.is_some());
        assert!(editor.busy);
        editor.cancel_analysis();
        wait_with_limit(&mut editor, Duration::from_secs(60));
        assert!(editor.analysis_pending.is_none());
        assert!(!editor.busy);
        assert_eq!(editor.status, "Analysis cancelled");
        assert_eq!(editor.document.project, before_cancel);
        assert!(editor.ready.is_some());
    });
}

fn state() -> (Editor, mpsc::Sender<Message>, mpsc::Receiver<Command>) {
    let (commands, receiver) = mpsc::channel();
    let (send, messages) = mpsc::channel();
    let mut editor = Editor::state(commands, messages, Arc::new(AtomicU64::new(0)));
    let id = editor
        .document
        .register(
            Source {
                id: SourceId(0),
                path: "scene.ARW".into(),
                development: SourceDevelopment {
                    dimensions: [824, 624],
                    active_area: [12, 12, 800, 600],
                    camera_model: "test-camera".into(),
                    camera_to_ap1: glam::DMat3::IDENTITY.to_cols_array_2d(),
                    input_floor: crate::processing::CAMERA_INPUT_FLOOR,
                },
                reference_only: false,
                photo: PhotoSettings::default(),
                orientation: Default::default(),
                crop: None,
            },
            Development::new(crate::raw::version(), [2., 1., 1.7, 1.]),
        )
        .unwrap();
    editor.id = 5;
    editor.next_request = 5;
    editor.current_source = Some(id);
    editor.floor = crate::processing::CAMERA_INPUT_FLOOR;
    editor.roll.gamma = 0.9;
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
    (editor, send, receiver)
}
fn pending(editor: &mut Editor) -> u64 {
    let id = editor.request_id();
    let revision = editor.analysis_generation.fetch_add(1, Ordering::Relaxed) + 1;
    editor.analysis_pending = Some(Pending {
        id,
        request: Request {
            loaded: editor.id,
            revision,
            document: editor.document.clone(),
            selected: editor.current_source.unwrap(),
            film: Some(rect([12, 12, 800, 600])),
            local_support: false,
            picture: Some(rect([52, 42, 720, 540])),
        },
    });
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
        .analysis_pending
        .as_ref()
        .unwrap()
        .request
        .document
        .clone();
    document.project.dmin = Some([0.8, 0.4, 0.2]);
    document.project.dmax = Some([0.04, 0.02, 0.01]);
    document.project.roll.balance = Balance {
        warmth: 0.1,
        tint: -0.05,
    };
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
    let (mut editor, send, _) = state();
    let mut second = editor.document.project.sources[0].clone();
    second.id = SourceId(0);
    second.path = "second.ARW".into();
    second.photo.exposure = 0.;
    editor.document.project.add_source(second).unwrap();
    let before = editor.document.project.clone();
    editor.viewport.zoom = Some(1.7);
    let id = pending(&mut editor);
    send.send(Message::AnalysisProgress(id, 1, 1)).unwrap();
    editor.receive();
    assert_eq!(editor.document.project, before);
    assert!(editor.busy);
    let result = outcome(&editor);
    let expected = result.document.project.clone();
    send.send(Message::Analyzed(id, Ok(Some(Box::new(result)))))
        .unwrap();
    editor.receive();
    assert!(!editor.busy && editor.analysis_pending.is_none());
    assert_eq!(editor.dmin.unwrap(), DVec3::new(0.8, 0.4, 0.2));
    assert_eq!(editor.dmax.unwrap(), DVec3::new(0.04, 0.02, 0.01));
    assert_eq!(editor.roll.exposure, 2.1);
    assert_eq!(editor.roll.gamma, before.roll.gamma);
    assert_eq!(editor.roll.response, before.roll.response);
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
    assert_eq!(editor.document.project.development, before.development);
    assert_eq!(editor.document.project.camera_basis, before.camera_basis);
    assert_eq!(editor.viewport.zoom, Some(1.7));
    assert!(!editor.source);
    assert!(editor.document.dirty());
    editor.roll.exposure = 0.;
    editor.photo.exposure = 0.;
    editor.photo.wb = Balance {
        warmth: 0.25,
        tint: -0.15,
    };
    let manual_photo = editor.photo;
    editor.sync_document();
    editor.receive();
    assert_eq!(editor.roll.exposure, 0.); // Manual zero is not an auto-placement trigger.
    assert_eq!(editor.photo, manual_photo);
}
#[test]
fn failure_cancel_and_invalid_gpu_combination_preserve_all_processing_state() {
    for mode in 0..3 {
        let (mut editor, send, _) = state();
        let before = editor.document.project.clone();
        let id = pending(&mut editor);
        let mut result = outcome(&editor);
        if mode == 1 {
            editor.cancel_analysis();
        }
        if mode == 2 {
            result.document.project.roll.gamma = 0.1;
            result.document.project.roll.balance = Balance {
                warmth: 0.8,
                tint: 0.8,
            };
            result.document.project.roll.response = crate::processing::Response::Film {
                toe: 1.,
                shoulder: 0.02,
            };
        }
        send.send(Message::Analyzed(
            id,
            if mode == 0 {
                Err("sampling failed".into())
            } else {
                Ok(Some(Box::new(result)))
            },
        ))
        .unwrap();
        editor.receive();
        assert_eq!(editor.document.project, before);
        assert_eq!(editor.roll, before.roll);
        assert_eq!(editor.photo, before.sources[0].photo);
        assert!(!editor.busy);
    }
}
#[test]
fn stale_request_generation_source_document_and_controls_cannot_install() {
    for mode in 0..5 {
        let (mut editor, send, _) = state();
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
                editor.roll.gamma = 1.;
                editor.sync_document();
            }
            _ => {
                editor.document.project.sources[0].path = "replacement.ARW".into();
            }
        }
        let before = editor.document.project.clone();
        send.send(Message::Analyzed(id + 1, Ok(None))).unwrap();
        editor.receive();
        assert!(editor.busy);
        send.send(Message::Analyzed(id, Ok(Some(Box::new(result)))))
            .unwrap();
        editor.receive();
        assert_eq!(editor.document.project, before);
    }
}
#[test]
fn area_accept_cancel_and_invalid_edits_do_not_change_calibration_or_exposure() {
    let (mut editor, _, _) = state();
    let original = editor.document.project.clone();
    editor.begin_pick(Pick::Analysis);
    editor.region = Some(Coverage::rectangle(rect([52, 42, 720, 540])));
    editor.finish_pick();
    assert_eq!(editor.document.project, original);
    editor.begin_pick(Pick::Analysis);
    editor.region = Some(Coverage::rectangle(rect([0, 0, 900, 700])));
    editor.apply_pick();
    assert_eq!(editor.document.project, original);
    assert_eq!(editor.pick, Some(Pick::Analysis));
    editor.region = Some(Coverage::rectangle(rect([52, 42, 720, 540])));
    editor.apply_pick();
    assert!(editor.pick.is_none());
    assert_eq!(
        editor.document.project.analysis_area,
        Some([52, 42, 720, 540])
    );
    assert_eq!(editor.roll, original.roll);
    assert_eq!(editor.document.project.dmin, original.dmin);
    assert_eq!(editor.document.project.dmax, original.dmax);
}
#[test]
fn failed_or_cancelled_analysis_keeps_settled_pixels_or_restarts_cancelled_pending_refinement() {
    for (settled, outstanding) in [(false, true), (true, true), (true, false)] {
        for cancel in [false, true] {
            let (mut editor, send, receiver) = super::super::tests::fixture();
            super::super::tests::add_ready_fixture(&mut editor);
            editor.source = false;
            let ctx = egui::Context::default();
            let frame = |editor: &mut Editor| {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1100., 800.),
                        )),
                        ..Default::default()
                    },
                    |ui| host::App::ui(editor, ui),
                );
                output.textures_delta.clear();
            };
            frame(&mut editor);
            editor.last_edit = std::time::Instant::now() - Duration::from_secs(1);
            frame(&mut editor);
            let revision = editor.generation.load(Ordering::Relaxed);
            let signature = editor.signature;
            let view = signature.unwrap().2.unwrap();
            let image = editor.ready.as_ref().unwrap().images[0].clone();
            assert!(editor.refining.is_some());
            if !outstanding {
                send.send(Message::Refined(revision, Ok(Some(image.clone()))))
                    .unwrap();
                editor.receive();
            } else if settled {
                editor.settled = Some((revision, view, image.clone()));
            }
            receiver.try_iter().for_each(drop);
            let before = editor.document.project.clone();
            editor.begin_analysis();
            assert!(editor.refining.is_none());
            assert_eq!(editor.signature, signature);
            assert_eq!(editor.settled.is_some(), settled);
            if outstanding {
                // A late completion from cancelled work cannot replace the accepted image.
                send.send(Message::Refined(revision, Ok(Some(image.clone()))))
                    .unwrap();
                editor.receive();
            } else {
                assert_eq!(editor.generation.load(Ordering::Relaxed), revision);
            }
            assert_eq!(editor.settled.is_some(), settled);
            frame(&mut editor);
            let pending = editor.analysis_pending.as_ref().unwrap().id;
            if cancel {
                editor.cancel_analysis();
            }
            send.send(Message::Analyzed(
                pending,
                if cancel {
                    Ok(None)
                } else {
                    Err("sampling failed".into())
                },
            ))
            .unwrap();
            editor.receive();
            assert_eq!(editor.document.project, before);
            frame(&mut editor);
            if settled {
                let (current_revision, current_view, retained) = editor.settled.as_ref().unwrap();
                assert_eq!(*current_revision, editor.generation.load(Ordering::Relaxed));
                assert_eq!(*current_view, view);
                assert!(Arc::ptr_eq(retained, &image));
                assert!(editor.refining.is_none());
                assert!(
                    !receiver
                        .try_iter()
                        .any(|command| matches!(command, Command::Refine(..)))
                );
            } else {
                assert!(editor.refining.is_some());
                assert!(
                    receiver
                        .try_iter()
                        .any(|command| matches!(command, Command::Refine(..)))
                );
            }
        }
    }
}

#[test]
fn statistics_edit_bypasses_composition_and_crop_drafts_cannot_start_analysis() {
    let (mut editor, _, receiver) = super::super::tests::fixture();
    editor.roll_orientation.turn_clockwise();
    editor.photo_orientation.flips.horizontal = true;
    editor.crop = Some(crate::crop::Crop {
        center: [444., 300.],
        size: [360, 240],
        angle: 0.1,
    });
    super::super::tests::add_ready_fixture(&mut editor);
    editor.source = false;
    editor.sync_document();
    let before = editor.document.project.clone();
    editor.viewport.zoom = Some(1.7);
    let view = editor.viewport;
    editor.begin_pick(Pick::Analysis);
    let area = editor.ready.as_ref().unwrap().area;
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100., 800.),
            )),
            ..Default::default()
        },
        |ui| host::App::ui(&mut editor, ui),
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
    editor.apply_pick();
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
    assert!(editor.crop_edit.is_some());
    editor.begin_analysis();
    assert!(editor.analysis_pending.is_none());
    assert!(
        !receiver
            .try_iter()
            .any(|command| matches!(command, Command::Analyze(..)))
    );
    editor.finish_crop(false);
}

#[test]
fn missing_local_support_never_becomes_whole_camera_picture_statistics() {
    let (editor, _, _) = state();
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
fn resetting_an_old_automatic_area_keeps_the_grade_and_requires_new_support() {
    let (mut editor, _, _) = state();
    editor.document.project.analysis_area = Some([52, 42, 720, 540]);
    let grade = editor.document.project.clone();
    editor.reset_analysis_area();
    assert_eq!(editor.document.project.analysis_area, None);
    assert_eq!(editor.roll, grade.roll);
    assert_eq!(editor.document.project.dmin, grade.dmin);
    assert_eq!(editor.document.project.dmax, grade.dmax);
    assert_eq!(editor.document.project.sources, grade.sources);
    assert_eq!(editor.analysis_picture(), None);
}

#[test]
fn sparse_eligibility_is_rejected_before_any_grade_installation() {
    let (mut editor, send, _) = state();
    let before = editor.document.project.clone();
    let id = pending(&mut editor);
    let samples = crate::scan::AnalysisSamples {
        pixels: vec![DVec3::splat(0.1)],
        attempted: 8192,
        rejected_capture: 8191,
        ..Default::default()
    };
    let error = require_picture(&samples).unwrap_err();
    send.send(Message::Analyzed(id, Err(error.to_string())))
        .unwrap();
    editor.receive();
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
        let mut editor = Editor::new(&device, &queue, None);
        let mut paths: Vec<_> = [4321, 4322, 4323]
            .iter()
            .map(|n| root.join(format!("DSC0{n}.ARW")))
            .collect();
        paths.extend((4312..=4320).map(|n| root.join(format!("DSC0{n}.ARW"))));
        editor.add_raws(paths);
        wait(&mut editor);
        assert!(editor.ready.is_some(), "{}", editor.status);
        assert_eq!(editor.ready.as_ref().unwrap().analysis_support, None);
        assert_eq!(editor.analysis_picture(), None);
        let before = editor.document.project.clone();
        click_analyze(&mut editor);
        wait(&mut editor);
        assert!(
            editor.status.starts_with("Analyzed 12"),
            "{}",
            editor.status
        );
        assert_eq!(editor.roll.gamma, before.roll.gamma);
        assert_eq!(editor.roll.response, before.roll.response);
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
        let dmax = editor.dmax.unwrap();
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
            editor.dmin,
            editor.roll.balance,
            editor.roll.exposure
        );
    });
}

fn ready_state() -> (Editor, mpsc::Sender<Message>, mpsc::Receiver<Command>) {
    let (mut editor, send, receiver) = super::super::tests::fixture();
    super::super::tests::add_ready_fixture(&mut editor);
    (editor, send, receiver)
}

fn click_label(editor: &mut Editor, ctx: &egui::Context, label: &str) {
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

fn click_analyze(editor: &mut Editor) {
    let ctx = egui::Context::default();
    click_label(editor, &ctx, "Analyze roll");
    if editor.analysis_confirmation.is_some() {
        click_label(editor, &ctx, "Analyze roll");
    }
}

#[test]
fn complete_ordered_photo_settings_determine_confirmation_including_inactive_controls() {
    for field in 0..7 {
        let (mut editor, _send, receiver) = ready_state();
        let mut other = editor.document.project.sources[0].clone();
        other.id = SourceId(0);
        other.path = "other.ARW".into();
        match field {
            0 => other.photo.exposure = 0.1,
            1 => other.photo.wb.warmth = 0.1,
            2 => other.photo.contrast = 1.1,
            3 => other.photo.pivot = 0.3,
            4 => other.photo.contrast_colour_mix = 0.4,
            5 => other.photo.saturation = 1.1,
            _ => other.photo.saturation_density = 0.4,
        }
        editor.document.project.add_source(other).unwrap();
        editor.sync_document();
        let before = editor.document.project.clone();
        let revision = editor.analysis_generation.load(Ordering::Relaxed);
        editor.request_analysis();
        assert!(editor.analysis_confirmation.is_some(), "field {field}");
        assert!(!editor.busy && editor.analysis_pending.is_none());
        assert!(receiver.try_recv().is_err());
        editor.request_analysis(); // A second request cannot bypass the existing prompt.
        editor.finish_analysis_confirmation(false);
        assert_eq!(editor.document.project, before);
        assert_eq!(editor.analysis_generation.load(Ordering::Relaxed), revision);
        assert!(receiver.try_recv().is_err());
    }
}

#[test]
fn default_ordered_photos_analyze_directly_even_with_adjusted_retired_sources_and_roll() {
    let (mut editor, _send, receiver) = ready_state();
    let mut retired = editor.document.project.sources[0].clone();
    retired.id = SourceId(0);
    retired.path = "retired.ARW".into();
    retired.photo.contrast = 1.3;
    let retired = editor.document.project.add_source(retired).unwrap();
    editor.document.project.order.retain(|id| *id != retired);
    editor.roll.exposure = 1.;
    editor.roll.wb.tint = 0.2;
    editor.request_analysis();
    assert!(editor.analysis_confirmation.is_none() && editor.analysis_pending.is_some());
    assert!(matches!(receiver.try_recv().unwrap(), Command::Analyze(..)));
}

#[test]
fn stale_confirmation_cannot_authorize_changed_document_order_selection_or_settings() {
    for changed in 0..9 {
        let (mut editor, _send, receiver) = ready_state();
        editor.photo.contrast = 1.2;
        editor.request_analysis();
        match changed {
            0 => editor.id += 1,
            1 => editor.current_source = None,
            2 => editor.document.path = Some("other.antitone.json".into()),
            3 => editor.document.project.order.clear(),
            4 => editor.photo.pivot = 0.4,
            5 => editor.roll.gamma = 0.8,
            6 => {
                editor.analysis_generation.fetch_add(1, Ordering::Relaxed);
            }
            7 => editor.document.project.analysis_area = Some([52, 42, 720, 540]),
            _ => editor.pick = Some(Pick::Wb),
        }
        editor.sync_document();
        let before = editor.document.project.clone();
        editor.finish_analysis_confirmation(true);
        assert!(editor.analysis_confirmation.is_none() && editor.analysis_pending.is_none());
        assert!(!editor.busy && receiver.try_recv().is_err());
        assert_eq!(editor.document.project, before);
        assert!(editor.status.contains("confirmation discarded"));
    }
}

#[test]
fn analyze_ui_warns_only_for_adjusted_photos_and_cancel_or_accept_is_single_action() {
    for accept in [false, true] {
        let (mut editor, _send, receiver) = ready_state();
        let ctx = egui::Context::default();
        let fresh = super::super::tests::app_frame(&mut editor, &ctx, vec![]);
        assert!(!fresh.iter().any(|(label, _)| label.contains("reset Photo")
            || label.contains("Replace Photo")
            || label.contains("replace existing Photo")));
        editor.photo.pivot = 0.3; // Contrast 1 is identity, but the complete settings are adjusted.
        click_label(&mut editor, &ctx, "Analyze roll");
        assert!(editor.analysis_confirmation.is_some());
        let before = editor.document.project.clone();
        let revision = editor.analysis_generation.load(Ordering::Relaxed);
        let labels = super::super::tests::app_frame(&mut editor, &ctx, vec![]);
        assert!(
            labels
                .iter()
                .any(|(label, _)| label == "Replace Photo adjustments?")
        );
        assert!(
            labels
                .iter()
                .any(|(label, _)| label.contains("Crop, orientation and flat-field correction"))
        );
        assert!(
            !receiver
                .try_iter()
                .any(|command| matches!(command, Command::Analyze(..)))
        );
        // Keyboard file/export/selection actions are unavailable behind the modal.
        super::super::tests::app_frame(
            &mut editor,
            &ctx,
            vec![
                egui::Event::Key {
                    key: egui::Key::S,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                },
                egui::Event::Key {
                    key: egui::Key::ArrowRight,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        assert!(!receiver.try_iter().any(|command| matches!(
            command,
            Command::Document(..) | Command::Analyze(..) | Command::Load(..)
        )));
        click_label(
            &mut editor,
            &ctx,
            if accept { "Analyze roll" } else { "Cancel" },
        );
        assert!(editor.analysis_confirmation.is_none());
        assert_eq!(editor.document.project, before);
        let count = receiver
            .try_iter()
            .filter(|command| matches!(command, Command::Analyze(..)))
            .count();
        assert_eq!(count, usize::from(accept));
        assert_eq!(
            editor.analysis_generation.load(Ordering::Relaxed),
            revision + u64::from(accept)
        );
        editor.finish_analysis_confirmation(true);
        assert!(receiver.try_recv().is_err());
    }
}

#[test]
fn analyze_decision_requires_explicit_acceptance_and_keyboard_cancel_keeps_accepted_state() {
    use super::super::presentation::tests::{key, pointer};
    for cancel_key in [egui::Key::Escape, egui::Key::Enter] {
        let (mut editor, _send, receiver) = ready_state();
        editor.photo.exposure = 1.25;
        editor.sync_document();
        let before = editor.document.project.clone();
        editor.request_analysis();
        let context = egui::Context::default();
        super::super::tests::app_frame(&mut editor, &context, vec![]);
        super::super::tests::app_frame(&mut editor, &context, vec![]);
        for pressed in [true, false] {
            super::super::tests::app_frame(
                &mut editor,
                &context,
                pointer(egui::pos2(30., 15.), pressed),
            );
        }
        assert!(editor.analysis_confirmation.is_some());
        assert!(receiver.try_recv().is_err());
        super::super::tests::app_frame(&mut editor, &context, key(cancel_key));
        assert!(editor.analysis_confirmation.is_none() && editor.analysis_pending.is_none());
        assert!(!editor.busy);
        assert_eq!(editor.document.project, before);
        assert!(receiver.try_recv().is_err());
    }
}

#[test]
fn photo_numeric_focus_commit_precedes_analyze_confirmation_snapshot() {
    let (mut editor, _send, receiver) = ready_state();
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
    super::super::tests::app_frame(&mut editor, &ctx, vec![]);
    super::super::tests::app_frame(
        &mut editor,
        &ctx,
        vec![
            egui::Event::PointerMoved(egui::pos2(950., 550.)),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                phase: egui::TouchPhase::Move,
                delta: egui::vec2(0., -200.),
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    for _ in 0..60 {
        super::super::tests::app_frame(&mut editor, &ctx, vec![]);
    }
    let labels = super::super::tests::app_frame(&mut editor, &ctx, vec![]);
    let exposure = labels
        .iter()
        .find(|(label, _)| label == "Exposure")
        .unwrap()
        .1;
    let point = labels
        .iter()
        .filter(|(label, rect)| {
            label.contains("0.00") && rect.min.y >= exposure.min.y && rect.min.x >= exposure.min.x
        })
        .min_by(|(_, a), (_, b)| a.min.y.total_cmp(&b.min.y))
        .unwrap()
        .1
        .center();
    for pressed in [true, false] {
        super::super::tests::app_frame(
            &mut editor,
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
    super::super::tests::app_frame(
        &mut editor,
        &ctx,
        vec![egui::Event::Text("0.7654321".into())],
    );
    assert_eq!(editor.photo.exposure, 0.);
    click_label(&mut editor, &ctx, "Roll");
    assert_eq!(
        editor.photo.exposure, 0.7654321,
        "tab={} status={}",
        editor.tab, editor.status
    );
    click_label(&mut editor, &ctx, "Analyze roll");
    let snapshot = &editor.analysis_confirmation.as_ref().unwrap().document;
    assert_eq!(
        snapshot
            .project
            .source(editor.current_source.unwrap())
            .unwrap()
            .photo
            .exposure,
        0.7654321
    );
    assert!(editor.analysis_pending.is_none());
    assert!(
        !receiver
            .try_iter()
            .any(|command| matches!(command, Command::Analyze(..)))
    );
}

#[test]
fn escape_cancels_confirmation_without_invalidating_settled_preview_or_grade() {
    let (mut editor, _send, receiver) = ready_state();
    editor.photo.contrast = 1.2;
    editor.source = false;
    let ctx = egui::Context::default();
    super::super::tests::app_frame(&mut editor, &ctx, vec![]);
    let signature = editor.signature;
    let revision = editor.generation.load(Ordering::Relaxed);
    let view = signature.unwrap().2.unwrap();
    let image = editor.ready.as_ref().unwrap().images[0].clone();
    editor.settled = Some((revision, view, image.clone()));
    editor.request_analysis();
    let before = editor.document.project.clone();
    super::super::tests::app_frame(&mut editor, &ctx, vec![]);
    super::super::tests::app_frame(
        &mut editor,
        &ctx,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert!(editor.analysis_confirmation.is_none() && editor.analysis_pending.is_none());
    assert_eq!(editor.document.project, before);
    assert_eq!(editor.signature, signature);
    assert_eq!(editor.generation.load(Ordering::Relaxed), revision);
    assert!(Arc::ptr_eq(&editor.settled.as_ref().unwrap().2, &image));
    assert!(
        !receiver
            .try_iter()
            .any(|command| matches!(command, Command::Analyze(..)))
    );
}

fn wait(editor: &mut Editor) {
    wait_with_limit(editor, Duration::from_secs(180));
}
fn wait_with_limit(editor: &mut Editor, limit: Duration) {
    let start = std::time::Instant::now();
    while editor.busy {
        assert!(start.elapsed() < limit, "{}", editor.status);
        std::thread::sleep(Duration::from_millis(10));
        editor.receive();
    }
}
