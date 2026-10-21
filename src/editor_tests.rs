use super::*;
#[path = "editor_viewer_tests.rs"]
mod viewer;
#[path = "editor_workspace_tests.rs"]
mod workspace;

pub(super) fn fixture() -> Editor {
    let mut editor = Editor::state();
    editor.dmin = Some(DVec3::new(0.2, 0.15, 0.1));
    editor.dmax = Some(DVec3::new(0.008, 0.005, 0.002));
    editor.floor = crate::processing::CAMERA_INPUT_FLOOR;
    editor.document.project.development =
        Some(Development::new(crate::raw::version(), [2., 1., 1.5, 1.]));
    editor.document.project.camera_basis = Some(crate::project::CameraBasis {
        camera_model: "test-camera".into(),
        camera_to_ap1: glam::DMat3::IDENTITY.to_cols_array_2d(),
    });
    editor
}

// Worker integration cases transfer owned state into a live application for the
// document operation, then recover it after its continuation has retired.
pub(super) fn complete_document_operation(editor: Editor, timeout: Duration) -> Editor {
    assert!(
        editor.document.write.request.is_some() || editor.candidate_operation.request.is_some()
    );
    assert!(editor.document.visit.measurement.is_none());
    assert!(!editor.document.visit.source.pending());
    let owner = std::cell::RefCell::new(Some(editor));
    {
        let mut app = crate::application::test_support::TestApplication::new(&owner, || {});
        app.enter(|editor, access| {
            editor.start_save_workflow(access);
            editor.start_document_workflow(access);
        });
        app.run_until(timeout, |editor| editor.document_pending.is_none());
        app.close_execution();
    }
    owner.into_inner().unwrap()
}

pub(super) fn complete_export_operation(editor: Editor, timeout: Duration) -> Editor {
    assert!(editor.export_operation.request.is_some());
    let owner = std::cell::RefCell::new(Some(editor));
    {
        let mut app = crate::application::test_support::TestApplication::new(&owner, || {});
        app.enter(|editor, access| editor.start_export_workflow(access));
        app.run_until(timeout, |editor| editor.export_operation.pending.is_none());
        app.close_execution();
    }
    owner.into_inner().unwrap()
}

pub(super) fn complete_reference_operation(editor: Editor, timeout: Duration) -> Editor {
    assert!(editor.flat_edit.as_ref().unwrap().request.is_some());
    assert!(editor.document.visit.measurement.is_none());
    let owner = std::cell::RefCell::new(Some(editor));
    {
        let mut app = crate::application::test_support::TestApplication::new(&owner, || {});
        app.enter(|editor, access| editor.start_reference_workflow(access));
        app.run_until(timeout, |editor| {
            !editor.busy && editor.document_pending.is_none()
        });
        app.close_execution();
    }
    owner.into_inner().unwrap()
}

pub(super) fn complete_analysis_preparation(editor: Editor, timeout: Duration) -> Editor {
    assert!(editor.document.analysis_preparing.is_some());
    complete_analysis_operation(editor, timeout)
}

pub(super) fn complete_analysis_operation(editor: Editor, timeout: Duration) -> Editor {
    assert!(
        editor.document.analysis_preparing.is_some()
            || editor.document.analysis_pending.is_some()
            || editor.document.visit.source.pending()
    );
    let owner = std::cell::RefCell::new(Some(editor));
    {
        let mut app = crate::application::test_support::TestApplication::new(&owner, || {});
        app.enter(|editor, access| {
            editor.start_analysis_preparation(access);
            editor.start_analysis_workflow(access);
            editor.start_source_workflow(access);
        });
        app.run_until(timeout, |editor| {
            editor.document.analysis_preparing.is_none()
                && editor.document.analysis_pending.is_none()
                && !editor.document.visit.source.pending()
                && !editor.busy
        });
        app.close_execution();
    }
    owner.into_inner().unwrap()
}

// Controller fixtures draw and inspect captured intent without starting workflows.
// Live fixtures enter the production UI and run its owning application between frames.
pub(in crate::editor) trait UiFixture {
    fn before_frame(&mut self) {}
    fn ui(&mut self, ui: &mut egui::Ui);
}

impl UiFixture for Editor {
    fn ui(&mut self, ui: &mut egui::Ui) {
        self.draw_ui(ui);
    }
}

pub(in crate::editor) fn paint_controller(editor: &mut impl UiFixture, ui: &mut egui::Ui) {
    editor.ui(ui);
}

impl UiFixture for crate::application::test_support::TestApplication<'_, Editor> {
    fn before_frame(&mut self) {
        self.pump();
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        self.enter(|editor, access| host::App::ui(editor, ui, access));
    }
}

#[test]
fn sdk_source_inspection_does_not_repeat_libraw_wb_headroom_compensation() {
    let mut editor = fixture();
    editor.document.project.development = Some(Development::dng([4., 1., 3., 1.]));
    let sdk = editor.source_parameters(0.).unwrap();
    let baseline = ProcessingParameters::source_preview(None, 0.).unwrap();
    assert_eq!(bytemuck::bytes_of(&sdk), bytemuck::bytes_of(&baseline));
    editor.document.project.development =
        Some(Development::new(crate::raw::version(), [4., 1., 3., 1.]));
    let native = editor.source_parameters(0.).unwrap();
    let compensated = ProcessingParameters::source_preview(Some([4., 1., 3., 1.]), 0.).unwrap();
    assert_eq!(
        bytemuck::bytes_of(&native),
        bytemuck::bytes_of(&compensated)
    );
    assert_ne!(bytemuck::bytes_of(&native), bytemuck::bytes_of(&sdk));
}

#[test]
fn refinement_worker_uses_the_submission_gate_supplied_to_the_host() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let mut editor = Editor::new(&device, &queue, None);
        editor.thumbnails = Thumbnails::default();
        editor.add_raws(vec![
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/roll-0.dng"),
        ]);
        editor = complete_document_operation(editor, Duration::from_secs(15));
        assert!(editor.document.visit.ready.is_some(), "{}", editor.status);
        let view = ViewRequest {
            area: editor.document.visit.ready.as_ref().unwrap().area,
            output: [16, 16],
            frame: None,
        };
        let revision = editor.original.revision();
        let submissions = host::App::submission_gate(&editor).clone();
        let request = original_view::Request {
            revision,
            generation: editor.original.generation.clone(),
            input: ProcessingParameters::source().into(),
            target: OutputTarget::Srgb,
            view,
            source: editor.source_context().unwrap(),
        };
        let services = editor.source_services.clone().unwrap();
        let gate = submissions.clone();
        let (completed, result) = mpsc::channel();
        let worker = submissions.configure(|| {
            let worker = std::thread::spawn(move || {
                completed
                    .send(pollster::block_on(request.prepare(services, gate)))
                    .unwrap();
            });
            assert!(matches!(
                result.recv_timeout(Duration::from_millis(100)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ));
            worker
        });
        let image = result
            .recv_timeout(Duration::from_secs(15))
            .unwrap()
            .unwrap()
            .expect("completed reconstruction");
        assert_eq!(image.processing_byte_len(), (0, 0));
        worker.join().unwrap();
    });
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
        let renderer = crate::renderer::GpuRenderer::new(&device, &queue)
            .await
            .unwrap();
        let proxy = crate::viewer::GpuProxy::new(
            renderer,
            &device,
            [16, 16],
            &[[1000, 1000, 1000, 0]; 256],
            glam::DMat3::IDENTITY,
            None,
            crate::gpu::SourceGeometry::new([12, 12, 8640, 5760], [16, 16]).unwrap(),
        )
        .unwrap();
        let area = Rect {
            x: 12,
            y: 12,
            width: 8640,
            height: 5760,
        };
        for (width, height, ppp) in [(1100., 800., 1.), (800., 600., 2.)] {
            for kind in [Pick::Flat, Pick::Dmin] {
                let mut editor = fixture();
                editor.dmin = None;
                editor.dmax = None;
                editor.source = true;
                editor.pick = Some(kind);
                editor.region = Some(Coverage::rectangle(Rect {
                    x: 46,
                    y: 46,
                    width: 8602,
                    height: 5508,
                }));
                let path = PathBuf::from(format!(
                    "/Users/example/{}/reference-with-a-long-filename.ARW",
                    "a-long-scanning-session-directory/".repeat(12)
                ));
                editor.document.visit.ready = Some(Ready {
                    developed: developed_fixture(),
                    file: thumbnails::FileKey::new(&path),
                    thumbnail: None,
                    path: path.clone(),
                    area,
                    analysis_support: None,
                    floor: editor.floor,
                    proxy: proxy.clone(),
                    record: Source {
                        id: SourceId(0),
                        path,
                        development: crate::project::SourceDevelopment {
                            lens_correction: None,
                            dng_development: None,
                            dimensions: [8704, 6144],
                            active_area: [12, 12, 8640, 5760],
                            camera_model: "test-camera".into(),
                            camera_to_ap1: glam::DMat3::IDENTITY.to_cols_array_2d(),
                            input_floor: editor.floor,
                        },
                        reference_only: true,
                        photo: Default::default(),
                        local_adjustments: None,
                        repairs: Default::default(),
                        orientation: Default::default(),
                        crop: None,
                    },
                    development: editor.document.project.development.clone().unwrap(),
                });
                let context = egui::Context::default();
                context.enable_accesskit();
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
                        |ui| editor.ui(ui),
                    );
                    output.textures_delta.clear();
                    let labels = ui_labels(output.clone());
                    fit_point = labels
                        .iter()
                        .find(|(name, _)| name == "Fit")
                        .map(|(_, rect)| rect.center());
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
                            egui::Shape::Text(text) if text.galley.job.text == "Cancel" => {
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
                    if frame > 0 {
                        assert!(image && rectangle && cancel);
                    } else {
                        assert!(image && rectangle);
                    }
                    assert!(editor.viewport.zoom.is_none());
                }
            }
        }
    });
}

#[test]
fn source_inspection_is_transient_and_measurement_inputs_exclude_its_gain() {
    let mut editor = fixture();
    add_ready_fixture(&mut editor);
    editor.sync_document();
    let project = editor.document.project.clone();
    let expected_dmin = editor.dmin;
    let expected_dmax = editor.dmax;
    let expected_roll = editor.roll;
    let expected_photo = editor.photo;

    editor.source_exposure = 2.3456789;
    editor.source = true;
    editor.sync_document();
    assert_eq!(editor.document.project, project);

    editor.begin_pick(Pick::Dmin);
    editor.region = Some(patch());
    let input = editor.measurement_input(Pick::Dmin, patch()).unwrap();
    assert_eq!(input.roll, expected_roll);
    assert_eq!(input.photo, expected_photo);
    editor.measurement_result(
        editor.document.visit.view,
        Pick::Dmin,
        Ok(Measurement::Reference(expected_dmin.unwrap())),
    );
    assert_eq!(editor.dmin, expected_dmin);
    assert_eq!(editor.dmax, expected_dmax);
    assert_eq!(editor.roll, expected_roll);
    assert_eq!(editor.photo, expected_photo);
    assert_eq!(editor.source_exposure, 2.3456789);
    editor.sync_document();
    assert_eq!(editor.document.project, project);
}

#[test]
fn every_picker_preserves_precise_edge_navigation_and_orientation_on_entry() {
    let mut editor = fixture();
    editor.roll_orientation.horizontal = true;
    editor.photo_orientation.turn_clockwise();
    let area = Rect {
        x: 12,
        y: 7,
        width: 8640,
        height: 5760,
    };
    let composition =
        Composition::new(area, editor.roll_orientation, editor.photo_orientation).unwrap();
    let canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900., 600.));
    let fit = editor.viewport.layout_composition(canvas, composition, 2.);
    editor
        .viewport
        .zoom_at(fit, 2., 2., fit.image.min + egui::vec2(10., 10.));
    editor.viewport.layout_composition(canvas, composition, 2.);
    let precise = editor.viewport;
    let geometry = (editor.roll_orientation, editor.photo_orientation);
    let grade = (editor.roll, editor.photo);
    for kind in [Pick::Dmin, Pick::Dmax, Pick::Gamma, Pick::Wb] {
        editor.begin_pick(kind);
        assert_eq!(editor.viewport, precise);
        assert_eq!(
            (editor.roll_orientation, editor.photo_orientation),
            geometry
        );
        assert_eq!((editor.roll, editor.photo), grade);
        editor.finish_pick();
        assert_eq!(editor.viewport, precise);
    }
}

#[test]
fn rejected_reference_and_cancel_restore_view_without_changing_processing() {
    let mut editor = fixture();
    editor.source = false;
    let original = (editor.dmin, editor.dmax, editor.roll, editor.photo);
    for (kind, invalid) in [(Pick::Dmin, 0.001), (Pick::Dmax, 1.)] {
        editor.begin_pick(kind);
        assert!(editor.source);
        editor.region = Some(patch());
        editor.measurement_result(
            editor.document.visit.view,
            kind,
            Ok(Measurement::Reference(DVec3::splat(invalid))),
        );
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
fn completing_initial_calibration_leaves_forced_source_view_but_preserves_explicit_inspection() {
    for order in [[Pick::Dmin, Pick::Dmax], [Pick::Dmax, Pick::Dmin]] {
        for initial_source in [None, Some(false), Some(true)] {
            let mut editor = fixture();
            if initial_source.is_none() {
                editor.dmin = None;
                editor.dmax = None;
            }
            editor.source = initial_source.unwrap_or(true);
            let settings = (editor.roll, editor.photo);
            for (index, kind) in order.into_iter().enumerate() {
                editor.begin_pick(kind);
                assert!(editor.source);
                editor.region = Some(patch());
                let value = DVec3::splat(if kind == Pick::Dmin { 0.2 } else { 0.005 });
                editor.measurement_result(
                    editor.document.visit.view,
                    kind,
                    Ok(Measurement::Reference(value)),
                );
                let expected = initial_source.unwrap_or(index == 0);
                assert_eq!(
                    editor.source, expected,
                    "{order:?}, initial={initial_source:?}, index={index}"
                );
                assert_eq!(
                    editor.calibration().is_some(),
                    initial_source.is_some() || index == 1
                );
                assert!(editor.pick.is_none() && !editor.busy);
                assert_eq!((editor.roll, editor.photo), settings);
            }
        }
    }
}

#[test]
fn failed_or_cancelled_initial_reference_picks_keep_forced_source_view_and_calibration() {
    for kind in [Pick::Dmin, Pick::Dmax] {
        let mut editor = fixture();
        if kind == Pick::Dmin {
            editor.dmin = None;
        } else {
            editor.dmax = None;
        }
        editor.source = true;
        let before = (
            editor.dmin,
            editor.dmax,
            editor.roll,
            editor.photo,
            editor.viewport,
        );
        editor.begin_pick(kind);
        editor.finish_pick();
        assert!(editor.source);
        for result in [
            Ok(Measurement::Reference(DVec3::splat(
                if kind == Pick::Dmin { 0.0001 } else { 0.4 },
            ))),
            Err("reference measurement failed".into()),
        ] {
            editor.begin_pick(kind);
            editor.region = Some(patch());
            editor.measurement_result(editor.document.visit.view, kind, result);
            assert!(editor.source);
            assert!(editor.pick.is_none() && !editor.busy && editor.calibration().is_none());
            assert_eq!(
                (
                    editor.dmin,
                    editor.dmax,
                    editor.roll,
                    editor.photo,
                    editor.viewport
                ),
                before
            );
        }
    }
}

#[test]
fn neutral_proposals_have_independent_scopes_and_reject_invalid_gpu_settings() {
    let mut editor = fixture();
    let id = editor
        .document
        .project
        .add_source(Source {
            id: SourceId(0),
            path: "test-camera.ARW".into(),
            development: crate::project::SourceDevelopment {
                lens_correction: None,
                dng_development: None,
                dimensions: [900, 600],
                active_area: [0, 0, 900, 600],
                camera_model: "test-camera".into(),
                camera_to_ap1: glam::DMat3::IDENTITY.to_cols_array_2d(),
                input_floor: editor.floor,
            },
            reference_only: false,
            photo: Default::default(),
            local_adjustments: None,
            repairs: Default::default(),
            orientation: Default::default(),
            crop: None,
        })
        .unwrap();
    editor.current_source = Some(id);
    editor.source = false;
    editor.photo = PhotoSettings {
        exposure: 0.5,
        wb: Balance {
            warmth: 0.2,
            tint: 0.1,
        },
        contrast: 0.4,
        pivot: 0.25,
        contrast_colour_mix: 0.75,
        saturation: 1.3,
        saturation_density: 0.25,
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
    editor.measurement_result(
        editor.document.visit.view,
        Pick::Gamma,
        Ok(Measurement::Balance(balance)),
    );
    assert_eq!(editor.roll.balance, balance);
    assert_eq!(editor.roll.gamma, gamma);
    assert_eq!(editor.roll.exposure, exposure);
    assert_eq!(editor.photo, photo);
    editor.begin_pick(Pick::RollWb);
    editor.measurement_result(
        editor.document.visit.view,
        Pick::RollWb,
        Ok(Measurement::Balance(balance)),
    );
    assert_eq!(editor.roll.wb, balance);
    assert_eq!(editor.roll.balance, balance);
    assert_eq!(editor.roll.gamma, gamma);
    assert_eq!(editor.roll.exposure, exposure);
    assert_eq!(editor.photo, photo);
    let roll = editor.roll;
    editor.begin_pick(Pick::Wb);
    editor.measurement_result(
        editor.document.visit.view,
        Pick::Wb,
        Ok(Measurement::Balance(balance)),
    );
    assert_eq!(editor.roll, roll);
    assert_eq!(
        editor.photo,
        PhotoSettings {
            wb: balance,
            ..photo
        }
    );
    editor.roll.gamma = 0.1;
    let old = editor.roll.balance;
    editor.begin_pick(Pick::Gamma);
    editor.measurement_result(
        editor.document.visit.view,
        Pick::Gamma,
        Ok(Measurement::Balance(Balance {
            warmth: 1.,
            tint: 1.,
        })),
    );
    assert_eq!(editor.roll.balance, old);
    assert!(editor.status.contains("rejected"));
    assert!(!editor.source);
}

#[test]
fn closed_application_clears_pick_busy_and_stale_samples_do_not_apply() {
    let mut editor = fixture();
    add_ready_fixture(&mut editor);
    editor.source = false;
    crate::application::test_support::with_application(editor, |app| {
        app.close_execution();
        app.enter(|editor, access| {
            editor.begin_pick(Pick::Dmin);
            editor.region = Some(patch());
            editor.apply_pick();
            editor.start_measurement_workflow(access);
            assert!(!editor.busy);
            assert!(editor.status.contains("stopped"));
            assert!(!editor.source);
            let old = editor.photo;
            let stale = editor.id + 1;
            editor.measurement_result(
                stale,
                Pick::Wb,
                Ok(Measurement::Balance(Balance {
                    warmth: 0.4,
                    tint: 0.4,
                })),
            );
            assert_eq!(editor.photo, old);
        });
    });
}

pub(super) struct ExecutionHold {
    release: mpsc::Sender<()>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Drop for ExecutionHold {
    fn drop(&mut self) {
        let _ = self.release.send(());
        self.worker.take().unwrap().join().unwrap();
    }
}

pub(super) fn hold_blocking_work(jobs: &crate::blocking_jobs::BlockingJobs) -> ExecutionHold {
    let jobs = jobs.clone();
    let (started, starts) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        futures_lite::future::block_on(jobs.run(move || {
            started.send(()).unwrap();
            let _ = released.recv();
        }))
        .unwrap();
    });
    starts.recv_timeout(Duration::from_secs(5)).unwrap();
    ExecutionHold {
        release,
        worker: Some(worker),
    }
}

// Geometry/presentation fixtures use authored native ownership but synthetic display geometry.
// Numerical measurement checks instead load coherent authored sources through production paths.
pub(super) fn developed_fixture() -> Arc<crate::scan::DevelopedSource> {
    let file = crate::raw::fixture::bayer_dng();
    Scan::open(file.path(), Some([1.; 4]))
        .unwrap()
        .source()
        .clone()
}

pub(super) fn add_ready_fixture(editor: &mut Editor) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let renderer = pollster::block_on(crate::renderer::GpuRenderer::new(&device, &queue)).unwrap();
    let proxy = crate::viewer::GpuProxy::new(
        renderer,
        &device,
        [1, 1],
        &[[1000, 1000, 1000, 0]],
        glam::DMat3::IDENTITY,
        None,
        crate::gpu::SourceGeometry::new([12, 7, 864, 576], [1, 1]).unwrap(),
    )
    .unwrap();
    let record = Source {
        id: SourceId(0),
        path: "current.ARW".into(),
        development: crate::project::SourceDevelopment {
            lens_correction: None,
            dng_development: None,
            dimensions: [900, 600],
            active_area: [12, 7, 864, 576],
            camera_model: "test-camera".into(),
            camera_to_ap1: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            input_floor: editor.floor,
        },
        reference_only: false,
        photo: editor.photo,
        local_adjustments: None,
        repairs: Default::default(),
        orientation: editor.photo_orientation,
        crop: editor.crop,
    };
    let development = editor.document.project.development.clone().unwrap();
    editor.current_source = Some(
        editor
            .document
            .register(record.clone(), development.clone())
            .unwrap(),
    );
    editor.document.visit.view = editor.id;
    editor.document.visit.ready = Some(Ready {
        developed: developed_fixture(),
        file: thumbnails::FileKey::new(&record.path),
        thumbnail: None,
        path: record.path.clone(),
        area: record.development.active_area.into(),
        analysis_support: None,
        floor: editor.floor,
        proxy,
        record,
        development,
    });
}

#[test]
fn startup_typography_preserves_an_already_completed_source_failure() {
    let mut editor = fixture();
    let context = egui::Context::default();
    context.options_mut(|options| options.max_passes = std::num::NonZeroUsize::new(1).unwrap());
    editor.busy = true;
    editor.status = "Loading source".into();
    editor.source_result(editor.id, Err("Source unavailable".into()));

    for initializing in [true, false] {
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            editor.ui(ui);
        });
        output.textures_delta.clear();
        assert!(editor.started);
        assert!(!editor.busy);
        assert_eq!(editor.status, "Source unavailable");
        if initializing {
            assert!(
                output.viewport_output[&egui::ViewportId::ROOT]
                    .commands
                    .iter()
                    .any(|command| matches!(command, egui::ViewportCommand::InnerSize(_)))
            );
        }
    }
}

pub(super) fn app_frame(
    editor: &mut impl UiFixture,
    context: &egui::Context,
    events: Vec<egui::Event>,
) -> Vec<(String, egui::Rect)> {
    editor.before_frame();
    presentation::configure_fonts(context);
    context.enable_accesskit();
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100., 800.),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| editor.ui(ui));
        },
    );
    output.textures_delta.clear();
    ui_labels(output)
}

pub(super) fn ui_labels(output: egui::FullOutput) -> Vec<(String, egui::Rect)> {
    let mut labels: Vec<_> = output
        .shapes
        .into_iter()
        .filter_map(|shape| {
            if let egui::Shape::Text(text) = shape.shape {
                Some((
                    text.galley.job.text.clone(),
                    text.galley.rect.translate(text.pos.to_vec2()),
                ))
            } else {
                None
            }
        })
        .collect();
    let painted: std::collections::HashSet<_> =
        labels.iter().map(|(label, _)| label.clone()).collect();
    if let Some(update) = output.platform_output.accesskit_update {
        for (_, node) in update.nodes {
            if let (Some(label), Some(bounds)) = (node.label(), node.bounds())
                && !painted.contains(label)
            {
                labels.push((
                    label.into(),
                    egui::Rect::from_min_max(
                        egui::pos2(bounds.x0 as f32, bounds.y0 as f32),
                        egui::pos2(bounds.x1 as f32, bounds.y1 as f32),
                    ),
                ));
            }
        }
    }
    labels
}

#[test]
fn crop_inspector_settings_remain_photo_only_without_transaction_buttons() {
    let mut editor = fixture();
    add_ready_fixture(&mut editor);
    let saved = (
        editor.roll,
        editor.photo,
        editor.crop,
        editor.roll_orientation,
        editor.photo_orientation,
    );
    editor.begin_crop();
    let draft = editor.document.visit.crop_edit.as_ref().unwrap().draft;
    for width in [270., 315.] {
        let context = egui::Context::default();
        for tab in [0, 1] {
            editor.tab = tab;
            for _ in 0..3 {
                controls_frame_sized(&mut editor, &context, width, 600., vec![]);
            }
            let text = controls_frame_sized(&mut editor, &context, width, 600., vec![]);
            let heading = if tab == 0 {
                "Reset scan orientation"
            } else {
                "Crop & straighten"
            };
            let hidden = if tab == 0 {
                "Crop & straighten"
            } else {
                "Reset scan orientation"
            };
            let title = text.iter().find(|(s, _)| s == heading).unwrap().1;
            let tabs = text.iter().find(|(s, _)| s == "Roll").unwrap().1;
            assert!(title.top() > tabs.bottom());
            assert!(!text.iter().any(|(s, _)| s == hidden));
            for label in ["Aspect", "Angle", "Thirds guides", "Reset draft"] {
                assert_eq!(
                    text.iter().any(|(s, _)| s == label),
                    tab == 1,
                    "{label} belongs only to Photo Composition"
                );
            }
            for label in ["Cancel", "Apply"] {
                assert!(
                    !text.iter().any(|(s, _)| s == label),
                    "{label} belongs to the viewer bar"
                );
            }
            let tabs_before = tabs;
            controls_frame_sized(
                &mut editor,
                &context,
                width,
                600.,
                vec![
                    egui::Event::PointerMoved(egui::pos2(width * 0.5, 350.)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        phase: egui::TouchPhase::Move,
                        delta: egui::vec2(0., -1000.),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            let text = controls_frame_sized(&mut editor, &context, width, 600., vec![]);
            assert_eq!(
                text.iter().find(|(s, _)| s == "Roll").unwrap().1,
                tabs_before
            );
        }
    }
    assert_eq!(
        editor.document.visit.crop_edit.as_ref().unwrap().draft,
        draft
    );
    assert_eq!(
        (
            editor.roll,
            editor.photo,
            editor.crop,
            editor.roll_orientation,
            editor.photo_orientation
        ),
        saved
    );
}

#[test]
fn tab_clicks_show_only_the_selected_scope_without_changing_view_or_crop_draft() {
    let mut editor = fixture();
    add_ready_fixture(&mut editor);
    editor.started = true;
    editor.tab = 1;
    editor.crop = Some(Crop::from_region(Rect {
        x: 200,
        y: 140,
        width: 127,
        height: 79,
    }));
    editor.photo_orientation.turn_clockwise();
    editor.roll_orientation.horizontal = true;
    editor.viewport.zoom = Some(2.);
    let context = egui::Context::default();
    for editing in [false, true] {
        if editing {
            editor.begin_crop();
        }
        for _ in 0..3 {
            app_frame(&mut editor, &context, vec![]);
        }
        let state = (
            editor.roll_orientation,
            editor.photo_orientation,
            editor.crop,
            editor.roll,
            editor.photo,
            editor.dmin,
            editor.dmax,
            editor.source,
            editor.target,
        );
        let viewport = editor.viewport;
        let signature = editor.original.signature.clone();
        let draft = editor
            .document
            .visit
            .crop_edit
            .as_ref()
            .map(|edit| edit.draft);
        let image = if editing {
            editor
                .document
                .visit
                .crop_edit
                .as_ref()
                .unwrap()
                .layout
                .unwrap()
                .image
        } else {
            editor.last_layout.unwrap().0.image
        };
        for (tab, label, heading, hidden) in [
            (0, "Roll", "Reset scan orientation", "Crop & straighten"),
            (1, "Photo", "Crop & straighten", "Reset scan orientation"),
            (2, "Rendering", "Target", "Crop & straighten"),
        ] {
            let labels = app_frame(&mut editor, &context, vec![]);
            let button = labels.iter().find(|(s, _)| s == label).unwrap().1.center();
            for pressed in [true, false] {
                app_frame(
                    &mut editor,
                    &context,
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
            let labels = app_frame(&mut editor, &context, vec![]);
            assert_eq!(editor.tab, tab);
            assert!(labels.iter().any(|(s, _)| s == heading));
            assert!(!labels.iter().any(|(s, _)| s == hidden));
            assert_eq!(
                (
                    editor.roll_orientation,
                    editor.photo_orientation,
                    editor.crop,
                    editor.roll,
                    editor.photo,
                    editor.dmin,
                    editor.dmax,
                    editor.source,
                    editor.target
                ),
                state
            );
            assert_eq!(editor.viewport, viewport);
            assert_eq!(editor.original.signature, signature);
            assert_eq!(
                editor
                    .document
                    .visit
                    .crop_edit
                    .as_ref()
                    .map(|edit| edit.draft),
                draft
            );
            let next_image = if editing {
                editor
                    .document
                    .visit
                    .crop_edit
                    .as_ref()
                    .unwrap()
                    .layout
                    .unwrap()
                    .image
            } else {
                editor.last_layout.unwrap().0.image
            };
            assert_eq!(next_image, image);
        }
    }
}

#[test]
fn crop_explicit_zoom_can_leave_the_frame_offscreen_without_requesting_refinement() {
    let mut editor = fixture();
    editor.crop = Some(Crop::from_region(Rect {
        x: 20,
        y: 15,
        width: 20,
        height: 15,
    }));
    add_ready_fixture(&mut editor);
    editor.started = true;
    editor.begin_crop();
    let context = egui::Context::default();
    for _ in 0..3 {
        app_frame(&mut editor, &context, vec![]);
    }
    let layout = editor
        .document
        .visit
        .crop_edit
        .as_ref()
        .unwrap()
        .layout
        .unwrap();
    assert!(
        layout
            .canvas
            .shrink(19.)
            .contains_rect(layout.screen_region(layout.composition.area()))
    );
    let zoom = layout.scale * context.pixels_per_point();
    let text = app_frame(&mut editor, &context, vec![]);
    let button = text.iter().find(|(s, _)| s == "100%").unwrap().1.center();
    for pressed in [true, false] {
        app_frame(
            &mut editor,
            &context,
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
    for _ in 0..3 {
        editor.original.last_edit = std::time::Instant::now() - Duration::from_secs(1);
        app_frame(&mut editor, &context, vec![]);
    }
    let explicit = editor
        .document
        .visit
        .crop_edit
        .as_ref()
        .unwrap()
        .layout
        .unwrap();
    assert!(!explicit.image.intersects(explicit.canvas));
    assert_eq!(explicit.scale * context.pixels_per_point(), 1.);
    assert!(editor.original.refining.is_none());
    assert!(editor.document.visit.grade.request.is_none());
    let text = app_frame(&mut editor, &context, vec![]);
    let fit = text
        .iter()
        .find(|(s, _)| s == "Fit source")
        .unwrap()
        .1
        .center();
    for pressed in [true, false] {
        app_frame(
            &mut editor,
            &context,
            vec![
                egui::Event::PointerMoved(fit),
                egui::Event::PointerButton {
                    pos: fit,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    let fitted = editor
        .document
        .visit
        .crop_edit
        .as_ref()
        .unwrap()
        .layout
        .unwrap();
    assert!(
        (fitted.scale * context.pixels_per_point() - zoom).abs() < 1e-6,
        "crop Fit changed: {layout:?} -> {fitted:?}; zoom={zoom}; widths={:?} heights={:?}",
        [layout.canvas.width(), fitted.canvas.width()],
        [layout.canvas.height(), fitted.canvas.height()]
    );
    assert!(
        fitted
            .canvas
            .shrink(19.)
            .contains_rect(fitted.screen_region(fitted.composition.area()))
    );
}

#[test]
fn crop_button_and_enter_escape_keys_apply_or_restore_the_starting_view() {
    let mut editor = fixture();
    add_ready_fixture(&mut editor);
    editor.started = true;
    editor.tab = 1;
    editor.viewport.zoom = Some(2.);
    let context = egui::Context::default();
    app_frame(&mut editor, &context, vec![]);
    for apply in [false, true] {
        app_frame(
            &mut editor,
            &context,
            vec![
                egui::Event::PointerMoved(egui::pos2(950., 450.)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: egui::vec2(0., 1000.),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        for _ in 0..12 {
            app_frame(&mut editor, &context, vec![]);
        }
        let labels = app_frame(&mut editor, &context, vec![]);
        let button = labels
            .iter()
            .find(|(label, _)| label == "Crop & straighten")
            .unwrap_or_else(|| panic!("crop control missing: {labels:?}"))
            .1
            .center();
        let viewport = editor.viewport;
        let crop = editor.crop;
        for pressed in [true, false] {
            app_frame(
                &mut editor,
                &context,
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
        assert!(editor.document.visit.crop_edit.is_some());
        let draft = Crop::from_region(Rect {
            x: 100,
            y: 70,
            width: 301,
            height: 203,
        });
        editor.document.visit.crop_edit.as_mut().unwrap().draft = draft;
        app_frame(
            &mut editor,
            &context,
            vec![egui::Event::Key {
                key: if apply {
                    egui::Key::Enter
                } else {
                    egui::Key::Escape
                },
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(editor.document.visit.crop_edit.is_none());
        if apply {
            assert_eq!(editor.crop, Some(draft));
            assert!(editor.viewport.zoom.is_none());
            let (layout, ppp) = editor.last_layout.unwrap();
            let expected =
                Viewport::default().layout_composition(layout.canvas, layout.composition, ppp);
            assert_eq!(layout.image, expected.image);
        } else {
            assert_eq!(editor.crop, crop);
            assert_eq!(editor.viewport, viewport);
        }
    }
}

#[test]
fn picker_toolbar_layout_does_not_rescale_a_fitted_composition() {
    let mut editor = fixture();
    editor.crop = Some(Crop::from_region(Rect {
        x: 150,
        y: 100,
        width: 500,
        height: 300,
    }));
    editor.photo_orientation.turn_clockwise();
    add_ready_fixture(&mut editor);
    editor.started = true;
    editor.source = false;
    let context = egui::Context::default();
    for kind in [Pick::Gamma, Pick::Wb, Pick::Dmin, Pick::Dmax] {
        for _ in 0..3 {
            app_frame(&mut editor, &context, vec![]);
        }
        let (before, _) = editor.last_layout.unwrap();
        let viewport = editor.viewport;
        let source = before.source_point(before.canvas.center());
        editor.begin_pick(kind);
        app_frame(&mut editor, &context, vec![]);
        let (after, _) = editor.last_layout.unwrap();
        assert_eq!(after.canvas, before.canvas);
        assert_eq!(after.scale, before.scale);
        assert!((after.source_point(after.canvas.center()) - source).length() < 0.002);
        editor.finish_pick();
        assert_eq!(editor.viewport, viewport);
        for _ in 0..3 {
            app_frame(&mut editor, &context, vec![]);
        }
        assert_eq!(
            editor.last_layout.unwrap().0.image,
            before.image,
            "{kind:?}: canvas {:?} -> {:?}",
            before.canvas,
            editor.last_layout.unwrap().0.canvas
        );
    }
}

#[test]
fn neutral_picker_exit_preserves_deliberate_navigation_not_its_initial_fit() {
    let mut editor = fixture();
    editor.crop = Some(Crop::from_region(Rect {
        x: 150,
        y: 100,
        width: 500,
        height: 300,
    }));
    add_ready_fixture(&mut editor);
    editor.started = true;
    let context = egui::Context::default();
    for kind in [Pick::Gamma, Pick::Wb] {
        editor.viewport.fit();
        for _ in 0..3 {
            app_frame(&mut editor, &context, vec![]);
        }
        editor.begin_pick(kind);
        app_frame(&mut editor, &context, vec![]);
        let (layout, ppp) = editor.last_layout.unwrap();
        editor
            .viewport
            .zoom_at(layout, ppp, 2., layout.canvas.center());
        app_frame(&mut editor, &context, vec![]);
        let (navigated, _) = editor.last_layout.unwrap();
        let anchor = navigated.source_point(navigated.canvas.center());
        editor.finish_pick();
        app_frame(&mut editor, &context, vec![]);
        let (returned, _) = editor.last_layout.unwrap();
        assert_eq!(editor.viewport.zoom, Some(2.));
        assert_eq!(returned.scale, navigated.scale);
        assert!((returned.source_point(returned.canvas.center()) - anchor).length() < 0.002);
    }
}

#[test]
fn crop_draft_cancel_and_apply_are_transactional_and_source_owned() {
    let mut editor = fixture();
    editor.crop = Some(Crop::from_region(Rect {
        x: 150,
        y: 100,
        width: 500,
        height: 300,
    }));
    add_ready_fixture(&mut editor);
    editor.sync_document();
    let directory = tempfile::tempdir().unwrap();
    *editor.document = editor
        .document
        .save(
            &directory.path().join("crop.json"),
            crate::project::Save::New,
        )
        .unwrap();
    let project = editor.document.project.clone();
    let crop = editor.crop;
    let grade = (editor.roll, editor.photo, editor.dmin, editor.dmax);
    editor.viewport.zoom = Some(2.);
    let viewport = editor.viewport;
    editor.begin_crop();
    let draft = Crop::from_region(Rect {
        x: 200,
        y: 120,
        width: 400,
        height: 220,
    });
    editor.document.visit.crop_edit.as_mut().unwrap().draft = draft;
    editor.sync_document();
    assert_eq!(editor.document.project, project);
    assert!(!editor.document.dirty());
    editor.file_action(files::FileAction::Save);
    assert!(editor.document.write.request.is_none());
    editor.finish_crop(false);
    assert_eq!(editor.viewport, viewport);
    assert_eq!(editor.crop, crop);
    editor.sync_document();
    assert_eq!(editor.document.project, project);
    assert!(!editor.document.dirty());
    editor.begin_crop();
    editor.document.visit.crop_edit.as_mut().unwrap().draft = draft;
    editor.finish_crop(true);
    editor.sync_document();
    assert_eq!(editor.crop, Some(draft));
    assert_eq!(
        editor
            .document
            .project
            .source(editor.current_source.unwrap())
            .unwrap()
            .crop,
        Some(draft)
    );
    assert!(editor.document.dirty());
    assert_eq!((editor.roll, editor.photo, editor.dmin, editor.dmax), grade);
    editor.begin_crop();
    editor.document.visit.crop_edit.as_mut().unwrap().draft =
        Crop::from_region(editor.document.visit.ready.as_ref().unwrap().area);
    editor.finish_crop(true);
    assert!(editor.crop.is_none());
}

#[test]
fn straightened_drafts_and_picks_are_transactional_and_measure_canvas_coverage() {
    let mut editor = fixture();
    let crop = Crop {
        center: [230.25, 170.75],
        size: [127, 79],
        angle: 0.271,
    };
    editor.crop = Some(crop);
    editor.roll_orientation.turn_clockwise();
    editor.photo_orientation.flips.horizontal = true;
    add_ready_fixture(&mut editor);
    let grade = (editor.roll, editor.photo, editor.dmin, editor.dmax);
    editor.begin_crop();
    let area = editor.document.visit.ready.as_ref().unwrap().area;
    editor.document.visit.crop_edit.as_mut().unwrap().draft =
        crop.straightened(area, -0.19).unwrap();
    editor.finish_crop(false);
    assert_eq!(editor.crop, Some(crop));
    editor.begin_crop();
    let draft = crop.straightened(area, -0.19).unwrap();
    editor.document.visit.crop_edit.as_mut().unwrap().draft = draft;
    editor.finish_crop(true);
    assert_eq!(editor.crop, Some(draft));
    editor.sync_document();
    assert_eq!(
        editor
            .document
            .project
            .source(editor.current_source.unwrap())
            .unwrap()
            .crop,
        Some(draft)
    );
    assert_eq!(grade, (editor.roll, editor.photo, editor.dmin, editor.dmax));
    let composition = Composition::with_crop(
        area,
        editor.roll_orientation,
        editor.photo_orientation,
        editor.crop,
    )
    .unwrap();
    let canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900., 600.));
    let layout = editor.viewport.layout_composition(canvas, composition, 2.);
    editor.last_layout = Some((layout, 2.));
    let viewport = editor.viewport;
    let anchor = layout.source_point(canvas.center());
    editor.begin_pick(Pick::Dmin);
    let full = Composition::new(area, editor.roll_orientation, editor.photo_orientation).unwrap();
    let uncropped = editor.viewport.layout_composition(canvas, full, 2.);
    assert_eq!(uncropped.scale, layout.scale);
    assert!((uncropped.source_point(canvas.center()) - anchor).length() < 0.002);
    editor.finish_pick();
    assert_eq!(editor.viewport, viewport);
    editor.last_layout = Some((layout, 2.));
    editor.begin_pick(Pick::Wb);
    let coverage = layout.pick_patch(canvas.center(), 32);
    let screen = layout.screen_coverage(coverage);
    assert!((screen.width() - 32. * layout.scale).abs() < 0.001);
    assert!((screen.height() - 32. * layout.scale).abs() < 0.001);
    let bounds = coverage.bounds(area).unwrap();
    assert!(bounds.width * bounds.height > 32 * 32);
    assert!((coverage.pixels(area).unwrap().map(|(_, w)| w).sum::<f64>() - 1024.).abs() < 1e-8);
    editor.region = Some(coverage);
    let input = editor.measurement_input(Pick::Wb, coverage).unwrap();
    assert_eq!(input.region, coverage);
    assert_eq!((input.roll, input.photo), (editor.roll, editor.photo));
}

#[test]
fn cropped_picker_views_preserve_source_anchor_and_restore_exact_working_view() {
    let mut editor = fixture();
    editor.crop = Some(Crop::from_region(Rect {
        x: 12,
        y: 7,
        width: 120,
        height: 90,
    }));
    editor.photo_orientation.turn_clockwise();
    editor.roll_orientation.horizontal = true;
    add_ready_fixture(&mut editor);
    editor.source = false;
    let area = editor.document.visit.ready.as_ref().unwrap().area;
    let composition = Composition::with_crop(
        area,
        editor.roll_orientation,
        editor.photo_orientation,
        editor.crop,
    )
    .unwrap();
    let canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900., 600.));
    let original = editor.viewport.layout_composition(canvas, composition, 2.);
    editor.last_layout = Some((original, 2.));
    let viewport = editor.viewport;
    let source_anchor = original.source_point(canvas.center());
    let state = (
        editor.crop,
        editor.roll,
        editor.photo,
        editor.dmin,
        editor.dmax,
    );
    for kind in [Pick::Dmin, Pick::Dmax, Pick::Gamma, Pick::Wb] {
        editor.begin_pick(kind);
        let measuring = if matches!(kind, Pick::Dmin | Pick::Dmax) {
            Composition::new(area, editor.roll_orientation, editor.photo_orientation).unwrap()
        } else {
            composition
        };
        let layout = editor.viewport.layout_composition(canvas, measuring, 2.);
        assert_eq!(layout.scale, original.scale);
        assert!((layout.source_point(canvas.center()) - source_anchor).length() < 0.002);
        if matches!(kind, Pick::Gamma | Pick::Wb) {
            assert_eq!(layout.image, original.image);
            assert!(!editor.source);
        } else {
            assert!(editor.source);
            assert_eq!(layout.area, area);
        }
        editor.finish_pick();
        assert_eq!(editor.viewport, viewport);
        assert_eq!(
            (
                editor.crop,
                editor.roll,
                editor.photo,
                editor.dmin,
                editor.dmax
            ),
            state
        );
    }
}

#[test]
fn cropped_export_snapshot_survives_later_crop_and_orientation_edits() {
    let mut editor = fixture();
    editor.crop = Some(Crop::from_region(Rect {
        x: 200,
        y: 140,
        width: 127,
        height: 79,
    }));
    editor.photo_orientation.turn_clockwise();
    editor.roll_orientation.horizontal = true;
    add_ready_fixture(&mut editor);
    editor.export(
        "crop.tif".into(),
        OutputFormat::Tiff(OutputTarget::Srgb),
        Overwrite::Refuse,
    );
    let recipe = editor.export_operation.request.take().unwrap().recipe;
    let (roll, photo, composition) = (recipe.roll, recipe.grade, recipe.composition);
    assert_eq!(composition.size(), [79, 127]);
    assert_eq!(composition.source_pixel([0, 0]).unwrap(), [326, 218]);
    assert_eq!((roll, photo.global()), (editor.roll, editor.photo));
    editor.crop = None;
    editor.photo_orientation = PhotoOrientation::default();
    assert_eq!(composition.size(), [79, 127]);
    assert_eq!(composition.source_pixel([0, 0]).unwrap(), [326, 218]);
}

#[test]
fn roll_reference_source_restores_working_orientation_and_precise_viewport() {
    let mut editor = fixture();
    editor.photo_orientation.turn_clockwise();
    editor.roll_orientation.horizontal = true;
    editor.roll_orientation.turn_clockwise();
    add_ready_fixture(&mut editor);
    let roll_orientation = editor.roll_orientation;
    let composition = Composition::new(
        editor.document.visit.ready.as_ref().unwrap().area,
        editor.roll_orientation,
        editor.photo_orientation,
    )
    .unwrap();
    let canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400., 300.));
    let fit = editor.viewport.layout_composition(canvas, composition, 2.);
    editor.viewport.zoom_at(fit, 2., 2., fit.image.max);
    editor.viewport.layout_composition(canvas, composition, 2.);
    let viewport = editor.viewport;
    let photo_orientation = editor.photo_orientation;
    let source_id = editor.current_source;
    editor.begin_pick(Pick::Dmin);
    assert_eq!(editor.viewport, viewport);
    let mut reference = editor.document.visit.ready.as_ref().unwrap().record.clone();
    reference.path = "reference.ARW".into();
    reference.orientation = PhotoOrientation::default();
    let development = editor
        .document
        .visit
        .ready
        .as_ref()
        .unwrap()
        .development
        .clone();
    let reference_id = editor.document.register(reference, development).unwrap();
    editor.pick_roll_source(reference_id);
    assert_eq!(editor.selected, 1);
    assert_eq!(editor.current_source, Some(reference_id));
    assert_eq!(editor.photo_orientation, PhotoOrientation::default());
    assert_eq!(editor.roll_orientation, roll_orientation);
    assert!(editor.document.visit.source.request.is_some());
    editor.finish_pick();
    assert!(editor.document.visit.source.request.is_some());
    assert_eq!(editor.current_source, source_id);
    assert_eq!(editor.selected, 0);
    assert_eq!(editor.document.project.order.len(), 2);
    assert_eq!(editor.roll_orientation, roll_orientation);
    assert_eq!(editor.photo_orientation, photo_orientation);
    assert_eq!(editor.viewport, viewport);
}

#[test]
fn export_owns_the_accepted_photo_snapshot_and_rejects_unpreviewable_locals() {
    use crate::{
        geometry::SourcePoint,
        local_adjustments::{GradientMask, LocalAdjustment},
        project::LocalAdjustments,
    };
    let mut editor = fixture();
    editor.photo.exposure = 1.25;
    add_ready_fixture(&mut editor);
    editor.export(
        "snapshot.tif".into(),
        OutputFormat::Tiff(OutputTarget::Srgb),
        Overwrite::Refuse,
    );
    let recipe = editor.export_operation.request.take().unwrap().recipe;
    let grade = recipe.grade;
    editor.photo.exposure = 7.;
    editor.sync_document();
    assert_eq!(grade.global().exposure, 1.25);
    assert!(grade.locals().is_empty());
    editor.busy = false;
    let id = editor.current_source.unwrap();
    let mut locals = LocalAdjustments::default();
    locals
        .add(
            "Linear 1".into(),
            LocalAdjustment {
                mask: GradientMask::Linear {
                    start: SourcePoint([0., 0.]),
                    end: SourcePoint([100., 0.]),
                },
                photo: PhotoSettings::default(),
                enabled: false,
                invert: false,
                strength: 1.,
            },
        )
        .unwrap();
    editor
        .document
        .project
        .source_mut(id)
        .unwrap()
        .local_adjustments = Some(locals);
    editor.export(
        "rejected.tif".into(),
        OutputFormat::Tiff(OutputTarget::Srgb),
        Overwrite::Refuse,
    );
    assert!(editor.export_operation.request.is_none());
    assert!(!editor.busy);
    assert!(editor.status.contains("local adjustments"));
}

#[test]
fn export_holds_the_accepted_orientation_snapshot() {
    let mut editor = fixture();
    editor.photo_orientation.turn_clockwise();
    editor.roll_orientation.horizontal = true;
    add_ready_fixture(&mut editor);
    editor.export(
        "output.tif".into(),
        OutputFormat::Tiff(OutputTarget::Srgb),
        Overwrite::Refuse,
    );
    let recipe = editor.export_operation.request.take().unwrap().recipe;
    let (roll, photo, composition) = (recipe.roll, recipe.grade, recipe.composition);
    assert_eq!(composition.size(), [576, 864]);
    assert_eq!(composition.source_pixel([0, 0]).unwrap(), [875, 582]);
    assert_eq!((roll, photo.global()), (editor.roll, editor.photo));
    editor.photo_orientation = PhotoOrientation::default();
    editor.roll_orientation = RollOrientation::default();
    assert_eq!(composition.size(), [576, 864]);
    assert_eq!(composition.source_pixel([0, 0]).unwrap(), [875, 582]);
}

#[test]
fn orientation_buttons_transform_displayed_photo_and_preserve_crop_and_other_scopes() {
    let mut editor = fixture();
    add_ready_fixture(&mut editor);
    editor.crop = Some(Crop::from_region(Rect {
        x: 200,
        y: 140,
        width: 127,
        height: 79,
    }));
    editor.roll_orientation.turn_counterclockwise();
    editor.tab = 1;
    let unchanged = (
        editor.crop,
        editor.roll_orientation,
        editor.roll,
        editor.photo,
        editor.dmin,
        editor.dmax,
    );
    let context = egui::Context::default();
    for label in [
        "Flip horizontal",
        "Turn right",
        "Flip vertical",
        "Turn left",
    ] {
        let before = Composition::with_crop(
            editor.document.visit.ready.as_ref().unwrap().area,
            editor.roll_orientation,
            editor.photo_orientation,
            editor.crop,
        )
        .unwrap();
        editor.viewport.zoom = Some(2.);
        click_section_button(&mut editor, &context, "Composition", label);
        assert!(editor.viewport.zoom.is_none());
        let after = Composition::with_crop(
            editor.document.visit.ready.as_ref().unwrap().area,
            editor.roll_orientation,
            editor.photo_orientation,
            editor.crop,
        )
        .unwrap();
        let size = before.size();
        let next = after.size();
        for p in [
            [0, 0],
            [next[0] - 1, 0],
            [0, next[1] - 1],
            [next[0] / 2, next[1] / 2],
        ] {
            let old = match label {
                "Flip horizontal" => [size[0] - 1 - p[0], p[1]],
                "Flip vertical" => [p[0], size[1] - 1 - p[1]],
                "Turn right" => [p[1], size[1] - 1 - p[0]],
                "Turn left" => [size[0] - 1 - p[1], p[0]],
                _ => unreachable!(),
            };
            assert_eq!(
                after.source_pixel(p).unwrap(),
                before.source_pixel(old).unwrap()
            );
        }
        assert_eq!(
            (
                editor.crop,
                editor.roll_orientation,
                editor.roll,
                editor.photo,
                editor.dmin,
                editor.dmax
            ),
            unchanged
        );
        editor.sync_document();
        assert_eq!(
            editor
                .document
                .project
                .source(editor.current_source.unwrap())
                .unwrap()
                .orientation,
            editor.photo_orientation
        );
    }
    assert!(editor.photo_orientation.is_identity());
}

#[test]
fn flip_buttons_are_repeatable_scoped_actions_and_disabled_when_busy_or_picking() {
    for section in ["Composition", "Scan orientation"] {
        let mut editor = fixture();
        add_ready_fixture(&mut editor);
        editor.tab = if section == "Composition" { 1 } else { 0 };
        editor.crop = Some(Crop::from_region(Rect {
            x: 200,
            y: 140,
            width: 127,
            height: 79,
        }));
        let context = egui::Context::default();
        let unchanged = (
            editor.crop,
            editor.roll,
            editor.photo,
            editor.dmin,
            editor.dmax,
        );
        for label in ["Flip horizontal", "Flip vertical"] {
            let before = (editor.roll_orientation, editor.photo_orientation);
            click_section_button(&mut editor, &context, section, label);
            let mut expected = before;
            match (section, label) {
                ("Composition", "Flip horizontal") => expected.1.flips.horizontal ^= true,
                ("Composition", _) => expected.1.flips.vertical ^= true,
                (_, "Flip horizontal") => expected.0.horizontal ^= true,
                _ => expected.0.vertical ^= true,
            }
            assert_eq!(
                (editor.roll_orientation, editor.photo_orientation),
                expected
            );
            editor.busy = true;
            click_section_button(&mut editor, &context, section, label);
            assert_eq!(
                (editor.roll_orientation, editor.photo_orientation),
                expected
            );
            editor.busy = false;
            editor.begin_pick(Pick::Wb);
            click_section_button(&mut editor, &context, section, label);
            assert_eq!(
                (editor.roll_orientation, editor.photo_orientation),
                expected
            );
            editor.finish_pick();
            click_section_button(&mut editor, &context, section, label);
            assert_eq!((editor.roll_orientation, editor.photo_orientation), before);
        }
        editor.begin_crop();
        let text = controls_frame(&mut editor, &context, 315., vec![]);
        let tabs = text.iter().find(|(label, _)| label == "Roll").unwrap().1;
        assert!(
            !text
                .iter()
                .any(|(label, rect)| label == "Flip horizontal" && rect.top() < tabs.top())
        );
        editor.tab = 0;
        click_section_button(&mut editor, &context, "Scan orientation", "Flip horizontal");
        assert!(editor.roll_orientation.is_identity());
        assert!(editor.photo_orientation.is_identity());
        assert_eq!(
            (
                editor.crop,
                editor.roll,
                editor.photo,
                editor.dmin,
                editor.dmax
            ),
            unchanged
        );
    }
}

#[test]
fn roll_turn_controls_preserve_photo_crop_and_grade_and_are_disabled_during_picking() {
    use crate::geometry::QuarterTurns;
    let mut editor = fixture();
    editor.roll_orientation.horizontal = true;
    editor.photo_orientation.turn_counterclockwise();
    editor.crop = Some(Crop::from_region(Rect {
        x: 200,
        y: 140,
        width: 127,
        height: 79,
    }));
    add_ready_fixture(&mut editor);
    editor.started = true;
    let unchanged = (
        editor.photo_orientation,
        editor.crop,
        editor.roll,
        editor.photo,
        editor.dmin,
        editor.dmax,
    );
    let context = egui::Context::default();
    for _ in 0..3 {
        app_frame(&mut editor, &context, vec![]);
    }
    let labels = app_frame(&mut editor, &context, vec![]);
    let button = labels
        .iter()
        .find(|(label, _)| label == "Turn right")
        .unwrap()
        .1
        .center();
    for pressed in [true, false] {
        app_frame(
            &mut editor,
            &context,
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
    let turned = RollOrientation {
        turns: QuarterTurns::One,
        horizontal: false,
        vertical: true,
    };
    assert_eq!(editor.roll_orientation, turned);
    assert_eq!(editor.document.project.orientation, turned);
    assert_eq!(
        (
            editor.photo_orientation,
            editor.crop,
            editor.roll,
            editor.photo,
            editor.dmin,
            editor.dmax
        ),
        unchanged
    );
    editor.begin_pick(Pick::Wb);
    for pressed in [true, false] {
        app_frame(
            &mut editor,
            &context,
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
    assert_eq!(editor.roll_orientation, turned);
    editor.finish_pick();
    let labels = app_frame(&mut editor, &context, vec![]);
    let reset = labels
        .iter()
        .find(|(label, _)| label == "Reset scan orientation")
        .unwrap()
        .1
        .center();
    for pressed in [true, false] {
        app_frame(
            &mut editor,
            &context,
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
    assert!(editor.roll_orientation.is_identity());
    assert_eq!(
        (
            editor.photo_orientation,
            editor.crop,
            editor.roll,
            editor.photo,
            editor.dmin,
            editor.dmax
        ),
        unchanged
    );
}

#[test]
fn shared_roll_turns_apply_to_new_photos_and_export_snapshots() {
    let mut editor = fixture();
    editor.roll_orientation.turn_clockwise();
    add_ready_fixture(&mut editor);
    let ready = editor.document.visit.ready.take().unwrap();
    editor.load("new.ARW".into());
    assert!(editor.document.visit.source.request.is_some());
    let mut record = ready.record;
    record.id = SourceId(0);
    record.path = "new.ARW".into();
    editor.source_result(
        editor.id,
        Ok(Box::new(Ready {
            developed: ready.developed,
            file: thumbnails::FileKey::new(&record.path),
            thumbnail: None,
            path: record.path.clone(),
            area: ready.area,
            analysis_support: None,
            floor: ready.floor,
            proxy: ready.proxy,
            record,
            development: ready.development,
        })),
    );
    assert_eq!(editor.document.project.sources.len(), 2);
    assert!(editor.photo_orientation.is_identity());
    assert_eq!(
        editor.roll_orientation.turns,
        crate::geometry::QuarterTurns::One
    );
    editor.export(
        "turned.tif".into(),
        OutputFormat::Tiff(OutputTarget::Srgb),
        Overwrite::Refuse,
    );
    let recipe = editor.export_operation.request.take().unwrap().recipe;
    let composition = recipe.composition;
    assert_eq!(composition.size(), [576, 864]);
    assert_eq!(composition.source_pixel([0, 0]).unwrap(), [12, 582]);
    editor.roll_orientation.turn_counterclockwise();
    assert_eq!(composition.size(), [576, 864]);
    assert_eq!(composition.source_pixel([0, 0]).unwrap(), [12, 582]);
}

#[test]
fn contextual_crop_reset_preserves_orientation_grade_and_accepted_state_until_apply() {
    let mut editor = fixture();
    editor.crop = Some(Crop::from_region(Rect {
        x: 100,
        y: 70,
        width: 301,
        height: 203,
    }));
    editor.photo_orientation.turn_clockwise();
    editor.roll_orientation.horizontal = true;
    add_ready_fixture(&mut editor);
    editor.tab = 1;
    let grade = (editor.roll, editor.photo, editor.dmin, editor.dmax);
    let context = egui::Context::default();
    let orientation = editor.photo_orientation;
    let crop = editor.crop;
    editor.begin_crop();
    click_control(&mut editor, &context, "Reset draft");
    assert_eq!(editor.crop, crop);
    assert_eq!(editor.photo_orientation, orientation);
    editor.finish_crop(true);
    assert_eq!(editor.photo_orientation, orientation);
    assert!(editor.crop.is_none());
    assert!(editor.roll_orientation.horizontal);
    assert_eq!((editor.roll, editor.photo, editor.dmin, editor.dmax), grade);
}

#[test]
fn navigation_invalidates_pending_overwrite() {
    let mut editor = fixture();
    editor.overwrite = Some((
        "existing.tif".into(),
        OutputFormat::Tiff(OutputTarget::DisplayP3),
    ));
    editor.load("another.arw".into());
    assert!(editor.overwrite.is_none());
    assert!(editor.document.visit.source.request.is_some());
}

fn patch() -> Coverage {
    Coverage::rectangle(Rect {
        x: 12,
        y: 12,
        width: 32,
        height: 32,
    })
}

#[test]
fn repicking_uses_current_selection_and_starts_without_a_rectangle() {
    let mut editor = fixture();
    for path in ["previous.arw", "current.arw"] {
        editor
            .document
            .register(
                Source {
                    id: SourceId(0),
                    path: path.into(),
                    development: crate::project::SourceDevelopment {
                        lens_correction: None,
                        dng_development: None,
                        dimensions: [900, 600],
                        active_area: [0, 0, 900, 600],
                        camera_model: "test-camera".into(),
                        camera_to_ap1: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                        input_floor: crate::processing::CAMERA_INPUT_FLOOR,
                    },
                    reference_only: false,
                    photo: PhotoSettings::default(),
                    local_adjustments: None,
                    repairs: Default::default(),
                    orientation: Default::default(),
                    crop: None,
                },
                Development::new(crate::raw::version(), [2., 1., 1.5, 1.]),
            )
            .unwrap();
    }
    editor.selected = 1;
    editor.current_source = Some(editor.document.project.order[editor.selected]);
    editor.region = Some(patch());
    editor.begin_pick(Pick::Dmin);
    assert_eq!(editor.selected, 1);
    assert!(editor.region.is_none());
    assert!(editor.document.visit.measurement.is_none());
    let value = DVec3::new(0.3, 0.2, 0.15);
    editor.measurement_result(
        editor.document.visit.view,
        Pick::Dmin,
        Ok(Measurement::Reference(value)),
    );
    assert_eq!(editor.dmin, Some(value));
    editor.document.project.order.clear();
    assert!(editor.calibration().is_some());
    editor.begin_pick(Pick::Dmax);
    assert!(editor.region.is_none());
    assert!(editor.document.visit.measurement.is_none());
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
    let full_scale_rect = rectangle(
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900., 600.)),
        area,
        egui::pos2(100., 200.),
        egui::pos2(300., 400.),
    );
    let half_scale_rect = rectangle(
        egui::Rect::from_min_size(egui::pos2(50., 20.), egui::vec2(450., 300.)),
        area,
        egui::pos2(100., 120.),
        egui::pos2(200., 220.),
    );
    assert_eq!(
        (
            full_scale_rect.x,
            full_scale_rect.y,
            full_scale_rect.width,
            full_scale_rect.height
        ),
        (
            half_scale_rect.x,
            half_scale_rect.y,
            half_scale_rect.width,
            half_scale_rect.height
        ),
    );
}

fn controls_frame(
    editor: &mut Editor,
    context: &egui::Context,
    width: f32,
    events: Vec<egui::Event>,
) -> Vec<(String, egui::Rect)> {
    controls_frame_sized(editor, context, width, 2000., events)
}

fn controls_frame_sized(
    editor: &mut Editor,
    context: &egui::Context,
    width: f32,
    height: f32,
    events: Vec<egui::Event>,
) -> Vec<(String, egui::Rect)> {
    presentation::configure_fonts(context);
    context.enable_accesskit();
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
    ui_labels(output)
}

fn click_section_header(editor: &mut Editor, context: &egui::Context, section: &str) {
    let text = controls_frame(editor, context, 315., vec![]);
    let position = text
        .iter()
        .find(|(label, _)| label == section)
        .unwrap()
        .1
        .center();
    for pressed in [true, false] {
        controls_frame(
            editor,
            context,
            315.,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
}

fn click_control(editor: &mut Editor, context: &egui::Context, label: &str) {
    for _ in 0..3 {
        controls_frame(editor, context, 315., vec![]);
    }
    let position = controls_frame(editor, context, 315., vec![])
        .into_iter()
        .filter(|(name, _)| name == label)
        .min_by(|(_, a), (_, b)| a.top().total_cmp(&b.top()))
        .unwrap_or_else(|| panic!("missing {label}"))
        .1
        .center();
    for pressed in [true, false] {
        controls_frame(
            editor,
            context,
            315.,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
}

fn click_section_button(editor: &mut Editor, context: &egui::Context, section: &str, label: &str) {
    if section == "Composition" || section == "Scan orientation" {
        click_control(editor, context, label);
        return;
    }
    for _ in 0..3 {
        controls_frame(editor, context, 315., vec![]);
    }
    let mut text = controls_frame(editor, context, 315., vec![]);
    let mut title = text.iter().find(|(s, _)| s == section).unwrap().1;
    if !text
        .iter()
        .any(|(s, r)| s == label && r.top() > title.bottom())
    {
        click_section_header(editor, context, section);
        text = controls_frame(editor, context, 315., vec![]);
        title = text.iter().find(|(s, _)| s == section).unwrap().1;
    }
    let button = text
        .iter()
        .find(|(s, r)| s == label && r.top() > title.bottom())
        .unwrap_or_else(|| panic!("missing {section}/{label}: {text:?}"))
        .1
        .center();
    for pressed in [true, false] {
        controls_frame(
            editor,
            context,
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
}

fn click_section_reset(editor: &mut Editor, context: &egui::Context, name: &str) {
    let text = controls_frame(editor, context, 315., vec![]);
    let title = text
        .iter()
        .find(|(label, _)| {
            label
                == if name == "Gamma balance" {
                    "Balance"
                } else {
                    name
                }
        })
        .unwrap()
        .1;
    let reset = text
        .iter()
        .find(|(label, rect)| {
            label == &format!("Reset {name}")
                && rect.y_range().contains(title.center().y)
                && rect.left() > title.right()
        })
        .unwrap_or_else(|| panic!("missing reset {name}: {text:?}"))
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
        let mut editor = fixture();
        for tab in [0, 1, 2] {
            editor.tab = tab;
            let text = controls_frame(&mut editor, &context, width, vec![]);
            let labels: Vec<_> = text.iter().map(|(label, _)| label.as_str()).collect();
            let required = if tab == 0 {
                vec![
                    "Reset scan orientation",
                    "Analyze",
                    "Auto-crop roll",
                    "Calibration",
                    "Gamma & balance",
                    "Balance",
                    "Toe & shoulder",
                    "Exposure",
                ]
            } else if tab == 1 {
                vec![
                    "Crop & straighten",
                    "Dust repair",
                    "Adjustments",
                    "Exposure",
                    "White balance",
                ]
            } else {
                vec!["Look", "None", "Target", "Display P3"]
            };
            for label in &required {
                assert!(labels.contains(label), "missing {label}");
            }
            if tab == 0 {
                assert!(!labels.contains(&"Toe"));
                assert!(!labels.contains(&"Shoulder"));
            }
            assert!(!labels.contains(&if tab == 0 {
                "Crop & straighten"
            } else {
                "Reset scan orientation"
            }));
            let heading = match tab {
                0 => "Reset scan orientation",
                1 => "Crop & straighten",
                _ => "Target",
            };
            let tabs = text.iter().find(|(label, _)| label == "Roll").unwrap().1;
            assert!(
                text.iter()
                    .find(|(label, _)| label == heading)
                    .unwrap()
                    .1
                    .top()
                    > tabs.bottom()
            );
            let sections = if tab == 0 {
                vec![
                    "Calibration",
                    "Gamma & balance",
                    "Toe & shoulder",
                    "Exposure",
                ]
            } else if tab == 1 {
                vec!["Crop & straighten", "Exposure", "White balance"]
            } else {
                vec!["Target"]
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
            }
        }
    }
}

#[test]
fn section_expansion_is_scope_specific_and_does_not_change_project_state() {
    let context = egui::Context::default();
    let mut editor = fixture();
    editor.roll.exposure = 1.23456789;
    editor.photo.exposure = -2.3456789;
    editor.sync_document();
    let before = editor.document.project.clone();
    let dirty = editor.document.dirty();
    let first = controls_frame(&mut editor, &context, 315., vec![]);
    assert!(
        first
            .iter()
            .any(|(label, _)| label == "Adjusted · +1.23 EV")
    );
    click_section_header(&mut editor, &context, "Exposure");
    let roll = controls_frame(&mut editor, &context, 315., vec![]);
    assert!(roll.iter().any(|(label, _)| label == "EV"));
    editor.tab = 1;
    click_section_header(&mut editor, &context, "Exposure");
    let photo = controls_frame(&mut editor, &context, 315., vec![]);
    assert!(
        photo
            .iter()
            .any(|(label, _)| label == "Adjusted · -2.35 EV")
    );
    editor.tab = 0;
    assert!(
        controls_frame(&mut editor, &context, 315., vec![])
            .iter()
            .any(|(label, _)| label == "EV")
    );
    editor.sync_document();
    assert_eq!(editor.document.project, before);
    assert_eq!(editor.document.dirty(), dirty);
    assert_eq!(editor.roll.exposure, 1.23456789);
    assert_eq!(editor.photo.exposure, -2.3456789);
}

#[test]
fn collapsed_calibration_keeps_missing_measurements_visible() {
    let context = egui::Context::default();
    let mut editor = fixture();
    editor.dmin = None;
    editor.dmax = None;
    click_section_header(&mut editor, &context, "Calibration");
    let text = controls_frame(&mut editor, &context, 315., vec![]);
    assert!(
        text.iter()
            .any(|(label, _)| label == "Dmin + Dmax not measured")
    );
    assert!(!text.iter().any(|(label, _)| label == "Pick Dmax"));
    editor.dmin = Some(glam::DVec3::splat(0.2));
    let text = controls_frame(&mut editor, &context, 315., vec![]);
    assert!(text.iter().any(|(label, _)| label == "Dmax not measured"));
    assert!(!text.iter().any(|(label, _)| label == "Pick Dmax"));
}

#[test]
fn crop_draft_controls_expand_below_the_fixed_photo_tool_shelf() {
    let context = egui::Context::default();
    let mut editor = fixture();
    add_ready_fixture(&mut editor);
    editor.tab = 1;
    assert!(
        controls_frame(&mut editor, &context, 315., vec![])
            .iter()
            .any(|(label, _)| label == "Crop & straighten")
    );
    let before = (
        editor.crop,
        editor.photo_orientation,
        editor.roll,
        editor.photo,
    );
    editor.begin_crop();
    for _ in 0..3 {
        app_frame(&mut editor, &context, vec![]);
    }
    let text = app_frame(&mut editor, &context, vec![]);
    for label in ["Aspect", "Angle", "Apply", "Cancel", "Thirds guides"] {
        assert!(
            text.iter().any(|(s, _)| s == label),
            "missing draft control {label}"
        );
    }
    editor.finish_crop(false);
    assert!(
        controls_frame(&mut editor, &context, 315., vec![])
            .iter()
            .any(|(label, _)| label == "Crop & straighten")
    );
    assert_eq!(
        (
            editor.crop,
            editor.photo_orientation,
            editor.roll,
            editor.photo
        ),
        before
    );
}

#[test]
fn section_and_color_resets_preserve_calibration_and_other_edit_scopes() {
    let context = egui::Context::default();
    let mut editor = fixture();
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
        ..PhotoSettings::default()
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
fn photographic_section_resets_are_scoped_and_disabled_while_busy() {
    let context = egui::Context::default();
    let mut editor = fixture();
    editor.tab = 1;
    for section in ["photo-composition", "photo-white-balance", "photo-exposure"] {
        editor.preferences.sections.insert(section.into(), false);
    }
    editor.photo = PhotoSettings {
        exposure: 1.25,
        wb: Balance {
            warmth: 0.2,
            tint: -0.1,
        },
        contrast: 1.2,
        pivot: 0.25,
        contrast_colour_mix: 1.,
        saturation: 1.3,
        saturation_density: 1.,
    };
    let photo = editor.photo;
    let roll = editor.roll;
    let calibration = (editor.dmin, editor.dmax);
    editor.busy = true;
    click_section_reset(&mut editor, &context, "Contrast");
    assert_eq!(editor.photo, photo);
    editor.busy = false;
    click_section_reset(&mut editor, &context, "Contrast");
    let defaults = PhotoSettings::default();
    assert_eq!(
        editor.photo,
        PhotoSettings {
            contrast: defaults.contrast,
            pivot: defaults.pivot,
            contrast_colour_mix: defaults.contrast_colour_mix,
            ..photo
        }
    );
    click_section_reset(&mut editor, &context, "Saturation");
    assert_eq!(
        editor.photo,
        PhotoSettings {
            exposure: photo.exposure,
            wb: photo.wb,
            ..defaults
        }
    );
    assert_eq!(editor.roll, roll);
    assert_eq!((editor.dmin, editor.dmax), calibration);
}

#[test]
fn response_section_is_reachable_by_scrolling_while_stage_tabs_stay_fixed() {
    for (width, height) in [(315., 800.), (270., 600.)] {
        let context = egui::Context::default();
        let mut editor = fixture();
        click_section_header(&mut editor, &context, "Toe & shoulder");
        for _ in 0..3 {
            controls_frame_sized(&mut editor, &context, width, height, vec![]);
        }
        let first = controls_frame_sized(&mut editor, &context, width, height, vec![]);
        let tabs = first.iter().find(|(label, _)| label == "Roll").unwrap().1;
        assert!(!first.iter().any(|(label, _)| label == "Target"));
        let grading_top = first
            .iter()
            .find(|(label, _)| label == "Roll")
            .unwrap()
            .1
            .bottom();
        let names = ["Toe & shoulder", "Toe", "Shoulder", "Exposure"];
        let mut visible = [false; 4];
        for frame in 0..150 {
            let events = if frame % 10 == 0 {
                vec![
                    egui::Event::PointerMoved(egui::pos2(width / 2., height - 40.)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        phase: egui::TouchPhase::Move,
                        delta: egui::vec2(0., -100.),
                        modifiers: egui::Modifiers::NONE,
                    },
                ]
            } else {
                vec![]
            };
            let text = controls_frame_sized(&mut editor, &context, width, height, events);
            for (i, name) in names.iter().enumerate() {
                if text.iter().any(|(label, rect)| {
                    label == name && rect.top() > grading_top && rect.bottom() < height - 8.
                }) {
                    visible[i] = true;
                }
            }
        }
        assert!(
            visible.into_iter().all(|seen| seen),
            "unreachable response controls: {visible:?}"
        );
        let scrolled = controls_frame_sized(&mut editor, &context, width, height, vec![]);
        assert_eq!(
            scrolled
                .iter()
                .find(|(label, _)| label == "Roll")
                .unwrap()
                .1,
            tabs
        );
    }
}

#[test]
fn original_grade_requires_explicit_response_opt_in_and_drawing_never_regrades_it() {
    let context = egui::Context::default();
    let mut editor = fixture();
    editor.roll.response = crate::processing::Response::Original;
    editor.roll.gamma = 0.73;
    editor.roll.exposure = 2.345;
    let before = editor.roll;
    let references = (editor.dmin, editor.dmax);
    let photo = editor.photo;
    click_section_header(&mut editor, &context, "Toe & shoulder");
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
