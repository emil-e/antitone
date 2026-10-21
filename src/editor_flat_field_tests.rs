use super::*;
use crate::project::SourceDevelopment;

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
fn flat_preview_cancel_failure_and_stale_products_preserve_document_and_working_context() {
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
    editor.begin_flat_edit("light.ARW".into(), None);
    let request = editor.id;
    assert!(matches!(
        editor.flat_edit.as_ref().unwrap().request,
        Some(reference_services::Operation::Preview(_))
    ));
    assert_eq!(editor.document.project, original);
    editor.reference_previewed(request, Err("reference unavailable".into()));
    assert!(editor.id > request);
    assert_eq!(editor.document.visit.view, editor.id);
    assert!(!editor.source);
    assert!(editor.pick.is_none());
    assert!(editor.flat_edit.is_none());
    assert_eq!(editor.document.project, original);
    assert_eq!(
        editor.failure.as_ref().unwrap().kind,
        FailureKind::FlatReference
    );
    assert_eq!(
        editor.failure.as_ref().unwrap().details,
        "reference unavailable"
    );
    assert!(editor.flat_edit.is_none());
    editor.begin_flat_edit("other.ARW".into(), None);
    let current = editor.id;
    editor.reference_previewed(request, Err("stale".into()));
    assert!(editor.busy);
    assert_eq!(editor.id, current);
    assert_eq!(
        editor.failure.as_ref().unwrap().details,
        "reference unavailable"
    );
    editor.busy = false;
    editor.finish_pick();
    assert!(editor.id > current);
    assert_eq!(editor.document.visit.view, editor.id);
    assert_eq!(editor.roll, roll);
    assert_eq!(editor.document.project, original);
    assert_eq!(
        editor.failure.as_ref().unwrap().details,
        "reference unavailable"
    );
}

#[test]
fn failed_map_acceptance_restores_transient_edit_without_accepting_document_changes() {
    let mut editor = Editor::state();
    editor.begin_flat_edit("light.ARW".into(), None);
    editor.busy = false;
    editor.floor = crate::processing::CAMERA_INPUT_FLOOR;
    editor.document.project.development =
        Some(Development::new(crate::raw::version(), [2., 1., 1.5, 1.]));
    super::super::tests::add_ready_fixture(&mut editor);
    let ready = editor.document.visit.ready.take().unwrap();
    editor.reference_previewed(
        editor.id,
        Ok(FlatPreview {
            ready,
            region: None,
        }),
    );
    let original = editor.document.project.clone();
    editor.region = Some(Coverage::rectangle(Rect {
        x: 12,
        y: 7,
        width: 1,
        height: 1,
    }));
    editor.apply_pick();
    let request = editor.id;
    assert!(matches!(
        editor.flat_edit.as_ref().unwrap().request,
        Some(reference_services::Operation::Accept(_))
    ));
    editor.document_prepared(request, Err("invalid correction".into()));
    assert!(editor.flat_edit.is_none());
    assert!(editor.pick.is_none());
    assert!(!editor.busy);
    assert_eq!(editor.document.project, original);
    assert_eq!(
        editor.failure.as_ref().unwrap().kind,
        FailureKind::FlatCorrection
    );
    assert_eq!(
        editor.failure.as_ref().unwrap().details,
        "invalid correction"
    );
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
    let mut record = ready.record.clone();
    record.path = "light.ARW".into();
    record.reference_only = true;
    let reference = Ready {
        developed: ready.developed.clone(),
        file: thumbnails::FileKey::new(&record.path),
        thumbnail: None,
        path: record.path.clone(),
        area,
        analysis_support: None,
        floor: ready.floor,
        proxy: ready.proxy.clone(),
        record,
        development: ready.development.clone(),
    };
    let region = Rect {
        x: 37,
        y: 26,
        width: 540,
        height: 320,
    };
    editor.begin_flat_edit(reference.path.clone(), Some(region));
    assert!(matches!(
        editor.flat_edit.as_ref().unwrap().request,
        Some(reference_services::Operation::Preview(_))
    ));
    editor.reference_previewed(
        editor.id,
        Ok(FlatPreview {
            ready: reference,
            region: None,
        }),
    );
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
    editor.apply_flat_edit();
    assert!(!editor.busy);
    assert!(
        editor
            .flat_edit
            .as_ref()
            .is_none_or(|edit| edit.request.is_none())
    );
    assert_eq!(editor.document.project, original);
    editor.region = Some(Coverage::rectangle(region));
    editor.apply_flat_edit();
    let request = editor.id;
    let Some(reference_services::Operation::Accept(input)) =
        editor.flat_edit.as_mut().unwrap().request.take()
    else {
        panic!("expected source support acceptance")
    };
    assert_eq!(input.region, region);
    assert!(Arc::ptr_eq(
        &input.source.source,
        &editor.document.visit.ready.as_ref().unwrap().developed
    ));
    assert!(input.source.correction.is_none());
    assert_eq!(input.document.project, original);
    assert_eq!(input.selected, editor.current_source);
    editor.document_prepared(request, Err("invalid correction".into()));
    assert_eq!(editor.document.project, original);
    assert_eq!(
        editor.crop,
        original
            .source(editor.current_source.unwrap())
            .unwrap()
            .crop
    );
    assert_eq!(editor.roll_orientation, original.orientation);
    assert_eq!(editor.viewport, viewport);
    assert!(!editor.source);
    assert!(editor.last_layout.is_none());
    assert!(
        editor
            .flat_edit
            .as_ref()
            .is_none_or(|edit| edit.request.is_none())
    );
    editor.begin_crop();
    let request = editor.id;
    editor.begin_flat_edit("another-light.ARW".into(), None);
    assert_eq!(editor.id, request);
    assert!(editor.document.visit.crop_edit.is_some());
    assert!(editor.flat_edit.is_none());
    assert!(
        editor
            .flat_edit
            .as_ref()
            .is_none_or(|edit| edit.request.is_none())
    );
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
        let mut editor = Editor::new(&device, &queue, None);
        // Thumbnail scheduling is separate and must not write the user's disk cache.
        editor.thumbnails = Thumbnails::default();
        editor.add_raws(vec![root.join("illuminated-patches.dng")]);
        editor = crate::editor::tests::complete_document_operation(editor, Duration::from_secs(60));
        assert!(editor.document.visit.ready.is_some(), "{}", editor.status);
        let selected = editor.current_source;
        let mut scan = Scan::open(&root.join("illuminated-patches.dng"), None).unwrap();
        editor.dmin = Some(
            scan.reference(Rect {
                x: 16,
                y: 16,
                width: 32,
                height: 32,
            })
            .unwrap(),
        );
        editor.dmax = Some(
            scan.reference(Rect {
                x: 80,
                y: 16,
                width: 32,
                height: 32,
            })
            .unwrap(),
        );
        editor.sync_document();
        let rgb = (editor.dmin, editor.dmax);
        let original = editor.document.project.clone();
        editor.begin_flat_edit(root.join("bare-light.dng"), None);
        editor =
            crate::editor::tests::complete_reference_operation(editor, Duration::from_secs(60));
        assert_eq!(editor.pick, Some(Pick::Flat));
        assert!(editor.region.is_some());
        editor.finish_pick();
        assert_eq!(editor.document.project, original);
        assert_eq!(editor.current_source, selected);
        editor.begin_flat_edit(root.join("bare-light.dng"), None);
        editor =
            crate::editor::tests::complete_reference_operation(editor, Duration::from_secs(60));
        editor.apply_pick();
        editor =
            crate::editor::tests::complete_reference_operation(editor, Duration::from_secs(60));
        assert!(editor.correction.is_some(), "{}", editor.status);
        assert_eq!(editor.document.project.order, original.order);
        assert_eq!((editor.dmin, editor.dmax), rgb);
        assert_eq!(editor.current_source, selected);
        let settings = editor.document.project.flat_field.clone().unwrap();
        assert!(
            editor
                .document
                .project
                .source(settings.reference)
                .unwrap()
                .reference_only
        );
        let directory = tempfile::tempdir().unwrap();
        let accepted = editor.document.project.clone();
        editor.restore_history(history::Direction::Undo);
        editor = crate::editor::tests::complete_document_operation(editor, Duration::from_secs(60));
        assert_eq!(editor.document.project, original, "{}", editor.status);
        assert!(editor.correction.is_none());
        editor.restore_history(history::Direction::Redo);
        editor = crate::editor::tests::complete_document_operation(editor, Duration::from_secs(60));
        assert_eq!(editor.document.project, accepted, "{}", editor.status);
        assert_eq!(
            editor.correction.as_ref().unwrap().settings.map,
            settings.map
        );
        editor.begin_flat_edit(directory.path().join("missing.dng"), None);
        assert!(editor.busy);
        assert!(editor.flat_edit.is_some());
        editor =
            crate::editor::tests::complete_reference_operation(editor, Duration::from_secs(60));
        assert!(editor.status.contains("open"), "{}", editor.status);
        assert!(!editor.busy);
        assert!(editor.flat_edit.is_none());
        assert!(editor.pick.is_none());
        assert!(editor.document_pending.is_none());
        assert_eq!(editor.document.project, accepted);
        assert_eq!(editor.current_source, selected);
        assert!(editor.document.visit.ready.is_some());
        let path = directory.path().join("authored-correction.antitone.json");
        editor.queue_save(path.clone(), crate::project::Save::New, None);
        editor = crate::editor::tests::complete_document_operation(editor, Duration::from_secs(60));
        assert!(!editor.document.dirty(), "{}", editor.status);
        editor.start_document(Preparation::Open(path));
        editor = crate::editor::tests::complete_document_operation(editor, Duration::from_secs(60));
        assert_eq!(editor.document.project, accepted);
        assert_eq!(
            editor.correction.as_ref().unwrap().settings.map,
            settings.map
        );
        let mut missing = accepted;
        missing.source_mut(settings.reference).unwrap().path =
            directory.path().join("unavailable.dng");
        let path = directory.path().join("missing-reference.antitone.json");
        missing.save(&path, crate::project::Save::New).unwrap();
        editor.start_document(Preparation::Open(path));
        editor = crate::editor::tests::complete_document_operation(editor, Duration::from_secs(60));
        assert_eq!(editor.document.project, missing);
        assert_eq!(
            editor.correction.as_ref().unwrap().settings.map,
            settings.map
        );
        assert!(editor.document.visit.ready.is_some(), "{}", editor.status);
    });
}
