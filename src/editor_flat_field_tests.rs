use super::*;
use crate::project::SourceDevelopment;

fn record(path: &str) -> Source {
    Source {
        id: SourceId(0),
        path: path.into(),
        development: SourceDevelopment {
            lens_correction: None,
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
fn flat_preview_cancel_failure_and_stale_messages_preserve_document_and_working_context() {
    let (commands, receiver) = mpsc::channel();
    let (send, messages) = mpsc::channel();
    let mut editor = Editor::state(
        commands,
        messages,
        Arc::new(std::sync::atomic::AtomicU64::new(0)),
    );
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
    assert!(matches!(receiver.recv().unwrap(), Command::FlatPreview(..)));
    assert_eq!(editor.document.project, original);
    send.send(Message::FlatPreview(
        request,
        Err("reference unavailable".into()),
    ))
    .unwrap();
    editor.receive();
    assert_eq!(editor.id, 12);
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
    receiver.recv().unwrap();
    editor.begin_flat_edit("other.ARW".into(), None);
    let current = editor.id;
    receiver.recv().unwrap();
    send.send(Message::FlatPreview(request, Err("stale".into())))
        .unwrap();
    editor.receive();
    assert!(editor.busy);
    assert_eq!(editor.id, current);
    assert_eq!(
        editor.failure.as_ref().unwrap().details,
        "reference unavailable"
    );
    editor.busy = false;
    editor.finish_pick();
    assert_eq!(editor.id, 12);
    assert_eq!(editor.roll, roll);
    assert_eq!(editor.document.project, original);
    assert_eq!(
        editor.failure.as_ref().unwrap().details,
        "reference unavailable"
    );
}

#[test]
fn failed_map_acceptance_restores_transient_edit_without_accepting_document_changes() {
    let (commands, receiver) = mpsc::channel();
    let (send, messages) = mpsc::channel();
    let mut editor = Editor::state(
        commands,
        messages,
        Arc::new(std::sync::atomic::AtomicU64::new(0)),
    );
    editor.begin_flat_edit("light.ARW".into(), None);
    receiver.recv().unwrap();
    editor.busy = false;
    editor.floor = crate::processing::CAMERA_INPUT_FLOOR;
    editor.document.project.development =
        Some(Development::new(crate::raw::version(), [2., 1., 1.5, 1.]));
    super::super::tests::add_ready_fixture(&mut editor);
    let original = editor.document.project.clone();
    editor.region = Some(Coverage::rectangle(Rect {
        x: 12,
        y: 7,
        width: 1,
        height: 1,
    }));
    editor.apply_pick();
    let request = editor.id;
    assert!(matches!(receiver.recv().unwrap(), Command::FlatAccept(..)));
    send.send(Message::Prepared(request, Err("invalid correction".into())))
        .unwrap();
    editor.receive();
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
    let (commands, receiver) = mpsc::channel();
    let (send, messages) = mpsc::channel();
    let mut editor = Editor::state(
        commands,
        messages,
        Arc::new(std::sync::atomic::AtomicU64::new(0)),
    );
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
        editor.ready.as_ref().unwrap().area,
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
    let ready = editor.ready.as_ref().unwrap();
    let area = ready.area;
    let mut record = ready.record.clone();
    record.path = "light.ARW".into();
    record.reference_only = true;
    let reference = Ready {
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
    assert!(matches!(receiver.recv().unwrap(), Command::FlatPreview(..)));
    send.send(Message::FlatPreview(
        editor.id,
        Ok(Box::new(FlatPreview {
            ready: reference,
            region: None,
        })),
    ))
    .unwrap();
    editor.receive();
    let context = egui::Context::default();
    let mut output = context.run_ui(
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
    assert!(receiver.try_recv().is_err());
    assert_eq!(editor.document.project, original);
    editor.region = Some(Coverage::rectangle(region));
    editor.apply_flat_edit();
    let Command::FlatAccept(request, support, document, selected) = receiver.recv().unwrap() else {
        panic!("expected source support acceptance")
    };
    assert_eq!(support, region);
    assert_eq!(document.project, original);
    assert_eq!(selected, editor.current_source);
    send.send(Message::Prepared(request, Err("invalid correction".into())))
        .unwrap();
    editor.receive();
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
    assert!(matches!(receiver.recv().unwrap(), Command::CancelFlat(..)));
    editor.begin_crop();
    let request = editor.id;
    editor.begin_flat_edit("another-light.ARW".into(), None);
    assert_eq!(editor.id, request);
    assert!(editor.crop_edit.is_some());
    assert!(editor.flat_edit.is_none());
    assert!(receiver.try_recv().is_err());
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
        wait_with_limit(&mut editor, Duration::from_secs(60));
        assert!(editor.ready.is_some(), "{}", editor.status);
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
        wait_with_limit(&mut editor, Duration::from_secs(60));
        assert_eq!(editor.pick, Some(Pick::Flat));
        assert!(editor.region.is_some());
        editor.finish_pick();
        assert_eq!(editor.document.project, original);
        assert_eq!(editor.current_source, selected);
        editor.begin_flat_edit(root.join("bare-light.dng"), None);
        wait_with_limit(&mut editor, Duration::from_secs(60));
        editor.apply_pick();
        wait_with_limit(&mut editor, Duration::from_secs(60));
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
        wait_with_limit(&mut editor, Duration::from_secs(60));
        assert_eq!(editor.document.project, original, "{}", editor.status);
        assert!(editor.correction.is_none());
        editor.restore_history(history::Direction::Redo);
        wait_with_limit(&mut editor, Duration::from_secs(60));
        assert_eq!(editor.document.project, accepted, "{}", editor.status);
        assert_eq!(
            editor.correction.as_ref().unwrap().settings.map,
            settings.map
        );
        editor.begin_flat_edit(directory.path().join("missing.dng"), None);
        assert!(editor.busy);
        assert!(editor.flat_edit.is_some());
        wait_with_limit(&mut editor, Duration::from_secs(60));
        assert!(editor.status.contains("open"), "{}", editor.status);
        assert!(!editor.busy);
        assert!(editor.flat_edit.is_none());
        assert!(editor.pick.is_none());
        assert!(editor.document_pending.is_none());
        assert_eq!(editor.document.project, accepted);
        assert_eq!(editor.current_source, selected);
        assert!(editor.ready.is_some());
        let path = directory.path().join("authored-correction.antitone.json");
        editor.queue_save(path.clone(), crate::project::Save::New, None);
        wait_with_limit(&mut editor, Duration::from_secs(60));
        assert!(!editor.document.dirty(), "{}", editor.status);
        editor.start_document(DocumentWork::Open(path));
        wait_with_limit(&mut editor, Duration::from_secs(60));
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
        editor.start_document(DocumentWork::Open(path));
        wait_with_limit(&mut editor, Duration::from_secs(60));
        assert_eq!(editor.document.project, missing);
        assert_eq!(
            editor.correction.as_ref().unwrap().settings.map,
            settings.map
        );
        assert!(editor.ready.is_some(), "{}", editor.status);
    });
}

fn wait_with_limit(editor: &mut Editor, limit: Duration) {
    let start = std::time::Instant::now();
    while editor.busy {
        editor.receive();
        assert!(start.elapsed() < limit, "{}", editor.status);
        std::thread::sleep(Duration::from_millis(10));
    }
}
