use super::*;
use crate::{application::test_support::with_application, project::SourceDevelopment};

fn record(path: &str) -> Source {
    Source {
        id: SourceId(0),
        path: path.into(),
        development: SourceDevelopment {
            lens_correction: None,
            dng_development: None,
            dimensions: [824, 624],
            active_area: [12, 12, 800, 600],
            camera_model: "test-camera".into(),
            camera_to_ap1: glam::DMat3::IDENTITY.to_cols_array_2d(),
            input_floor: 1. / 65535.,
        },
        reference_only: false,
        photo: Default::default(),
        local_adjustments: None,
        repairs: Default::default(),
        orientation: Default::default(),
        crop: None,
    }
}

#[test]
fn reference_registration_does_not_add_a_photo_or_hide_an_existing_one() {
    let mut document = Document::default();
    let development = Development::new(crate::raw::version(), [2., 1., 1.7, 1.]);
    let photo = document
        .register(record("photo.ARW"), development.clone())
        .unwrap();
    let same = document
        .register_reference(record("photo.ARW"), development.clone())
        .unwrap();
    assert_eq!(same, photo);
    assert_eq!(document.project.order, vec![photo]);
    assert!(!document.project.source(photo).unwrap().reference_only);
    let reference = document
        .register_reference(record("light.ARW"), development)
        .unwrap();
    assert_eq!(document.project.order, vec![photo]);
    assert!(document.project.source(reference).unwrap().reference_only);
}

#[test]
fn reference_rejection_cancel_and_stale_products_preserve_document_and_working_context() {
    let mut editor = Editor::state();
    editor.id = 12;
    editor.next_request = 12;
    editor.source = false;
    editor.roll.exposure = 2.123456789;
    editor.dmin = Some(DVec3::splat(0.2));
    editor.floor = 1. / 65535.;
    editor.sync_document();
    let original = editor.document.project.clone();
    let roll = editor.roll;
    with_application(editor, |app| {
        app.enter(|e, cx| {
            e.begin_flat_edit("light.ARW".into(), None, cx);
            let request = e.id;
            assert!(e.busy && e.flat_edit.as_ref().unwrap().task.is_some());
            assert_eq!(e.document.project, original);
            // This case tests the controller's typed rejection, not a native producer.
            e.reference_previewed(request, Err("reference unavailable".into()));
            assert!(e.id > request);
            assert_eq!(e.document.visit.view, e.id);
            assert!(!e.source && e.pick.is_none() && e.flat_edit.is_none());
            assert_eq!(e.document.project, original);
            assert_eq!(e.failure.as_ref().unwrap().kind, FailureKind::FlatReference);
            assert_eq!(e.failure.as_ref().unwrap().details, "reference unavailable");
            e.begin_flat_edit("other.ARW".into(), None, cx);
            let current = e.id;
            e.reference_previewed(request, Err("stale".into()));
            assert!(e.busy);
            assert_eq!(e.id, current);
            assert_eq!(e.failure.as_ref().unwrap().details, "reference unavailable");
            e.finish_pick(cx);
            assert!(e.id > current);
            assert_eq!(e.document.visit.view, e.id);
            assert_eq!(e.roll, roll);
            assert_eq!(e.document.project, original);
            assert_eq!(e.failure.as_ref().unwrap().details, "reference unavailable");
        });
        app.pump();
    });
}

#[test]
fn failed_map_acceptance_restores_transient_edit_without_accepting_document_changes() {
    let mut editor = Editor::state();
    editor.floor = crate::processing::CAMERA_INPUT_FLOOR;
    editor.document.project.development =
        Some(Development::new(crate::raw::version(), [2., 1., 1.5, 1.]));
    super::super::tests::add_ready_fixture(&mut editor);
    let ready = editor.document.visit.ready.as_ref().unwrap().clone();
    super::super::tests::paused_reference_fixture(&mut editor, ready, None);
    let original = editor.document.project.clone();
    editor.region = Some(Coverage::rectangle(Rect {
        x: 12,
        y: 7,
        width: 1,
        height: 1,
    }));
    with_application(editor, |app| {
        app.enter(|e, cx| {
            e.apply_flat_edit(cx);
            assert!(e.document_pending.is_some() && e.flat_edit.as_ref().unwrap().task.is_some());
            e.document_prepared(e.id, Err("invalid correction".into()));
        });
        app.pump();
        app.read(|e| {
            assert!(e.flat_edit.is_none() && e.pick.is_none() && !e.busy);
            assert_eq!(e.document.project, original);
            assert_eq!(
                e.failure.as_ref().unwrap().kind,
                FailureKind::FlatCorrection
            );
            assert_eq!(e.failure.as_ref().unwrap().details, "invalid correction");
        });
    });
}

#[test]
fn flat_support_uses_source_space_and_restores_rotated_composition_and_view() {
    let mut editor = Editor::state();
    editor.floor = crate::processing::CAMERA_INPUT_FLOOR;
    editor.document.project.development =
        Some(Development::new(crate::raw::version(), [2., 1., 1.5, 1.]));
    super::super::tests::add_ready_fixture(&mut editor);
    editor.dmin = Some(DVec3::splat(0.2));
    editor.dmax = Some(DVec3::splat(0.01));
    editor.source = false;
    editor.roll_orientation.turn_clockwise();
    editor.roll_orientation.horizontal = true;
    editor.photo_orientation.turn_counterclockwise();
    editor.photo_orientation.flips.vertical = true;
    editor.crop = Some(Crop {
        center: [400.25, 200.75],
        size: [300, 200],
        angle: 0.2,
    });
    editor.viewport.zoom = Some(1.375);
    let composition = Composition::with_crop(
        editor.document.visit.ready.as_ref().unwrap().area,
        editor.roll_orientation,
        editor.photo_orientation,
        editor.crop,
    )
    .unwrap();
    let canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(180., 120.));
    let layout = editor.viewport.layout_composition(canvas, composition, 1.);
    editor.viewport.pan(layout, 1., egui::vec2(24., -13.));
    editor.last_layout = Some((
        editor.viewport.layout_composition(canvas, composition, 1.),
        1.,
    ));
    editor.sync_document();
    let original = editor.document.project.clone();
    let viewport = editor.viewport;
    let ready = editor.document.visit.ready.as_ref().unwrap();
    let area = ready.area;
    let mut reference = ready.clone();
    reference.record.path = "light.ARW".into();
    reference.record.reference_only = true;
    reference.path = reference.record.path.clone();
    reference.file = thumbnails::FileKey::new(&reference.path);
    let region = Rect {
        x: 37,
        y: 26,
        width: 540,
        height: 320,
    };
    super::super::tests::paused_reference_fixture(&mut editor, reference, Some(region));
    let context = egui::Context::default();
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100., 800.),
            )),
            ..Default::default()
        },
        |ui| crate::editor::tests::paint_controller(&mut editor, ui),
    );
    output.textures_delta.clear();
    let layout = editor.last_layout.unwrap().0;
    assert_eq!(
        layout.composition,
        Composition::new(
            area,
            RollOrientation::default(),
            PhotoOrientation::default()
        )
        .unwrap()
    );
    assert_eq!(
        layout.frame_bounds(editor.region.unwrap()),
        (
            crate::geometry::FramePoint([25., 19.]),
            crate::geometry::FramePoint([565., 339.])
        )
    );
    assert_eq!(editor.document.project, original);
    editor.region = Some(editor.crop.unwrap().coverage().unwrap());
    editor.apply_flat_edit(&UiContext::render_only());
    assert!(!editor.busy && editor.document_pending.is_none());
    assert_eq!(editor.document.project, original);
    editor.region = Some(Coverage::rectangle(region));
    let input = editor.reference_input().unwrap().unwrap();
    assert_eq!(input.region, region);
    assert!(Arc::ptr_eq(
        &input.source.source,
        &editor.document.visit.ready.as_ref().unwrap().developed
    ));
    assert!(input.source.correction.is_none());
    assert_eq!(input.document.project, original);
    assert_eq!(input.selected, editor.current_source);
    assert!(
        !editor.busy && editor.document_pending.is_none(),
        "building input must not start an operation"
    );
    with_application(editor, |app| {
        app.enter(|e, cx| {
            e.apply_flat_edit(cx);
            e.document_prepared(e.id, Err("invalid correction".into()));
            assert_eq!(e.document.project, original);
            assert_eq!(
                e.crop,
                original.source(e.current_source.unwrap()).unwrap().crop
            );
            assert_eq!(e.roll_orientation, original.orientation);
            assert_eq!(e.viewport, viewport);
            assert!(!e.source && e.last_layout.is_none() && e.flat_edit.is_none());
            e.begin_crop();
            let request = e.id;
            e.begin_flat_edit("another-light.ARW".into(), None, cx);
            assert_eq!(e.id, request);
            assert!(e.document.visit.crop_edit.is_some() && e.flat_edit.is_none());
        });
        app.pump();
    });
}

#[test]
fn authored_reference_worker_cancels_applies_and_restores_without_reference_reread() {
    pollster::block_on(async {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw");
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance
            .request_adapter(&Default::default())
            .await
            .expect("authored correction worker test requires Metal");
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let mut editor = Editor::new(&device, &queue);
        editor.thumbnails = Thumbnails::default();
        with_application(editor, |app| {
            app.enter(|e, cx| e.add_raws(vec![root.join("illuminated-patches.dng")], cx));
            app.run_until(Duration::from_secs(60), |e| !e.busy);
            assert!(app.read(|e| e.document.visit.ready.is_some()));
            let selected = app.read(|e| e.current_source);
            let mut scan = Scan::open(&root.join("illuminated-patches.dng"), None).unwrap();
            let (rgb, original) = app.update(|e| {
                e.dmin = Some(
                    scan.reference(Rect {
                        x: 16,
                        y: 16,
                        width: 32,
                        height: 32,
                    })
                    .unwrap(),
                );
                e.dmax = Some(
                    scan.reference(Rect {
                        x: 80,
                        y: 16,
                        width: 32,
                        height: 32,
                    })
                    .unwrap(),
                );
                e.sync_document();
                ((e.dmin, e.dmax), e.document.project.clone())
            });
            app.enter(|e, cx| e.begin_flat_edit(root.join("bare-light.dng"), None, cx));
            app.run_until(Duration::from_secs(60), |e| !e.busy);
            app.enter(|e, cx| {
                assert_eq!(e.pick, Some(Pick::Flat));
                assert!(e.region.is_some());
                e.finish_pick(cx);
                assert_eq!(e.document.project, original);
                assert_eq!(e.current_source, selected);
            });
            app.enter(|e, cx| e.begin_flat_edit(root.join("bare-light.dng"), None, cx));
            app.run_until(Duration::from_secs(60), |e| !e.busy);
            app.enter(|e, cx| e.apply_flat_edit(cx));
            app.run_until(Duration::from_secs(60), |e| !e.busy);
            let settings = app.read(|e| {
                assert!(e.correction.is_some(), "{}", e.status);
                assert_eq!(e.document.project.order, original.order);
                assert_eq!((e.dmin, e.dmax), rgb);
                assert_eq!(e.current_source, selected);
                let settings = e.document.project.flat_field.clone().unwrap();
                assert!(
                    e.document
                        .project
                        .source(settings.reference)
                        .unwrap()
                        .reference_only
                );
                settings
            });
            let directory = tempfile::tempdir().unwrap();
            let accepted = app.read(|e| e.document.project.clone());
            app.enter(|e, cx| e.restore_history(history::Direction::Undo, cx));
            app.run_until(Duration::from_secs(60), |e| !e.busy);
            app.read(|e| {
                assert_eq!(e.document.project, original, "{}", e.status);
                assert!(e.correction.is_none());
            });
            app.enter(|e, cx| e.restore_history(history::Direction::Redo, cx));
            app.run_until(Duration::from_secs(60), |e| !e.busy);
            app.read(|e| {
                assert_eq!(e.document.project, accepted, "{}", e.status);
                assert_eq!(e.correction.as_ref().unwrap().settings.map, settings.map);
            });
            app.enter(|e, cx| e.begin_flat_edit(directory.path().join("missing.dng"), None, cx));
            assert!(app.read(|e| e.busy && e.flat_edit.as_ref().unwrap().task.is_some()));
            app.run_until(Duration::from_secs(60), |e| !e.busy);
            app.read(|e| {
                assert!(e.status.contains("open"), "{}", e.status);
                assert!(e.flat_edit.is_none() && e.pick.is_none() && e.document_pending.is_none());
                assert_eq!(e.document.project, accepted);
                assert_eq!(e.current_source, selected);
                assert!(e.document.visit.ready.is_some());
            });
            let path = directory.path().join("authored-correction.antitone.json");
            app.enter(|e, cx| e.save_project(path.clone(), crate::project::Save::New, None, cx));
            app.run_until(Duration::from_secs(60), |e| !e.busy);
            assert!(!app.read(|e| e.document.dirty()));
            app.enter(|e, cx| e.start_document(Preparation::Open(path), cx));
            app.run_until(Duration::from_secs(60), |e| !e.busy);
            app.read(|e| {
                assert_eq!(e.document.project, accepted);
                assert_eq!(e.correction.as_ref().unwrap().settings.map, settings.map);
            });
            let mut missing = accepted;
            missing.source_mut(settings.reference).unwrap().path =
                directory.path().join("unavailable.dng");
            let path = directory.path().join("missing-reference.antitone.json");
            missing.save(&path, crate::project::Save::New).unwrap();
            app.enter(|e, cx| e.start_document(Preparation::Open(path), cx));
            app.run_until(Duration::from_secs(60), |e| !e.busy);
            app.read(|e| {
                assert_eq!(e.document.project, missing);
                assert_eq!(e.correction.as_ref().unwrap().settings.map, settings.map);
                assert!(e.document.visit.ready.is_some(), "{}", e.status);
            });
        });
    });
}
