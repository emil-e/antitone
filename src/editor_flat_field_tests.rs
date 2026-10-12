use super::*;
use crate::export::ImageSize;
use crate::project::{Fingerprint, SourceDevelopment};

fn record(path: &str) -> Source {
    Source {
        id: SourceId(0),
        path: path.into(),
        fingerprint: Fingerprint {
            bytes: 1,
            blake3: "0".repeat(64),
        },
        development: SourceDevelopment {
            dimensions: [824, 624],
            active_area: [12, 12, 800, 600],
            camera_model: "test-camera".into(),
            camera_to_ap1: glam::DMat3::IDENTITY.to_cols_array_2d(),
            input_floor: 1. / 65535.,
        },
        reference_only: false,
        photo: Default::default(),
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
    receiver.recv().unwrap();
    editor.begin_flat_edit("other.ARW".into(), None);
    let current = editor.id;
    receiver.recv().unwrap();
    send.send(Message::FlatPreview(request, Err("stale".into())))
        .unwrap();
    editor.receive();
    assert!(editor.busy);
    assert_eq!(editor.id, current);
    editor.busy = false;
    editor.finish_pick();
    assert_eq!(editor.id, 12);
    assert_eq!(editor.roll, roll);
    assert_eq!(editor.document.project, original);
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
        thumbnail: None,
        path: record.path.clone(),
        area,
        floor: ready.floor,
        images: ready.images.clone(),
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
#[ignore = "read-only local RAW/Metal integration; run explicitly with supplied samples"]
fn real_reference_editor_save_open_and_corrected_exports() {
    pollster::block_on(async {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let negative = root.join("samples/DSC01737.ARW");
        let reference = root.join("samples/DSC01739.ARW");
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let mut editor = Editor::new(&device, &queue, None);
        editor.add_raws(vec![negative.clone()]);
        wait(&mut editor);
        assert!(editor.ready.is_some(), "{}", editor.status);
        let selected = editor.current_source.unwrap();
        editor.dmin = Some(DVec3::new(0.2, 0.15, 0.1));
        editor.dmax = Some(DVec3::new(0.008, 0.005, 0.002));
        editor.photo.exposure = 1.123456789012345;
        editor.sync_document();
        let rgb = (editor.dmin, editor.dmax);
        let order = editor.document.project.order.clone();
        let original = editor.document.project.clone();
        editor.begin_flat_edit(reference.clone(), None);
        wait(&mut editor);
        assert_eq!(editor.pick, Some(Pick::Flat));
        assert!(editor.region.is_some(), "{}", editor.status);
        editor.finish_pick();
        assert_eq!(editor.document.project, original);
        assert_eq!(editor.current_source, Some(selected));
        assert!(editor.ready.is_some());
        editor.begin_flat_edit(reference, None);
        wait(&mut editor);
        editor.apply_pick();
        wait(&mut editor);
        assert!(editor.correction.is_some(), "{}", editor.status);
        assert_eq!(editor.document.project.order, order);
        assert_eq!((editor.dmin, editor.dmax), rgb);
        let ctx = egui::Context::default();
        let revision = editor.generation.load(std::sync::atomic::Ordering::Relaxed);
        let rendered = Arc::new(
            GpuImage::new_rendered(&device, &queue, 16, 16, editor.target)
                .await
                .unwrap(),
        );
        editor.settled = Some((
            revision,
            ViewRequest {
                area: editor.ready.as_ref().unwrap().area,
                output: [16, 16],
                frame: None,
            },
            rendered,
        ));
        editor.last_edit = std::time::Instant::now() - Duration::from_secs(1);
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100., 800.),
                )),
                ..Default::default()
            },
            |ui| editor.filmstrip(ui),
        );
        let started = std::time::Instant::now();
        while editor
            .thumbnails
            .ready_count(&editor.document, editor.target)
            != 1
        {
            editor.thumbnails.receive(&editor.document);
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "corrected thumbnail did not complete"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let settings = editor.document.project.flat_field.clone().unwrap();
        assert!(
            editor
                .document
                .project
                .source(settings.reference)
                .unwrap()
                .reference_only
        );
        let map = settings.map.clone();
        let before = editor.document.project.clone();
        editor.begin_flat_edit(root.join("samples/missing-reference.ARW"), None);
        wait(&mut editor);
        assert_eq!(editor.document.project, before);
        assert!(editor.ready.is_some());
        assert_eq!(editor.current_source, Some(selected));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roll.antitone.json");
        editor.queue_save(path.clone(), crate::project::Save::New, None);
        wait(&mut editor);
        assert!(!editor.document.dirty(), "{}", editor.status);
        editor.start_document(DocumentWork::Open(path));
        wait(&mut editor);
        assert_eq!(editor.document.project, before);
        assert!(editor.correction.is_some());
        assert_eq!(editor.correction.as_ref().unwrap().settings.map, map);
        let mut missing_reference = editor.document.project.clone();
        missing_reference
            .source_mut(settings.reference)
            .unwrap()
            .path = dir.path().join("unavailable-reference.ARW");
        let missing_path = dir.path().join("missing-reference.antitone.json");
        missing_reference
            .save(&missing_path, crate::project::Save::New)
            .unwrap();
        editor.start_document(DocumentWork::Open(missing_path));
        wait(&mut editor);
        assert_eq!(editor.document.project, missing_reference);
        assert!(editor.correction.is_some());
        let mut scan = Scan::open(
            &negative,
            Some(
                editor
                    .document
                    .project
                    .development
                    .as_ref()
                    .unwrap()
                    .fixed_wb,
            ),
        )
        .unwrap();
        scan.set_flat_field(Some(map)).unwrap();
        let calibration = scan
            .processing_calibration(editor.calibration().unwrap())
            .unwrap();
        let pipeline = crate::processing::Pipeline::new(
            calibration,
            editor.roll,
            editor.photo,
            scan.camera_to_ap1,
        )
        .unwrap();
        let a = scan.area;
        let x = a.x + a.width / 2;
        let y = a.y + a.height / 2;
        let size = ImageSize {
            width: 32,
            height: 32,
        };
        let pixel = |dx, dy| {
            pipeline
                .apply(scan.pixel(x + dx, y + dy).unwrap())
                .map(|p| p.rgb)
        };
        let tiff = dir.path().join("corrected.tif");
        let exr = dir.path().join("corrected.exr");
        export::tiff(
            &tiff,
            size,
            OutputTarget::DisplayP3,
            Overwrite::Refuse,
            editor.roll.response,
            pixel,
        )
        .unwrap();
        export::exr(
            &exr,
            size,
            ExrPrecision::Full,
            Overwrite::Refuse,
            editor.roll.response,
            pixel,
        )
        .unwrap();
        let mut decoder = tiff::decoder::Decoder::new(std::fs::File::open(tiff).unwrap()).unwrap();
        assert_eq!(decoder.dimensions().unwrap(), (32, 32));
        let tiff::decoder::DecodingResult::U16(actual) = decoder.read_image().unwrap() else {
            panic!("not RGB16");
        };
        let input: Vec<_> = (0..32)
            .flat_map(|y| (0..32).map(move |x| pixel(x, y).unwrap()))
            .collect();
        let expected: Vec<_> = OutputTarget::DisplayP3
            .encode_u16(
                &crate::rendering::CpuRenderer::new(OutputTarget::DisplayP3)
                    .unwrap()
                    .render(&input)
                    .unwrap(),
            )
            .unwrap()
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(actual, expected);
        let image = exr::prelude::read_first_rgba_layer_from_file(
            exr,
            |resolution, _| vec![[0f32; 4]; resolution.width() * resolution.height()],
            |pixels, position, (r, g, b, a): (f32, f32, f32, f32)| {
                pixels[position.y() * 32 + position.x()] = [r, g, b, a]
            },
        )
        .unwrap();
        let expected = crate::color::ap1_to_ap0() * pixel(0, 0).unwrap();
        let actual = image.layer_data.channel_data.pixels[0];
        for c in 0..3 {
            assert_eq!(actual[c], expected[c] as f32);
        }
        let half_path = dir.path().join("corrected-half.exr");
        export::exr(
            &half_path,
            size,
            ExrPrecision::Half,
            Overwrite::Refuse,
            editor.roll.response,
            pixel,
        )
        .unwrap();
        let half_image = exr::prelude::read_first_rgba_layer_from_file(
            half_path,
            |resolution, _| vec![[0f32; 4]; resolution.width() * resolution.height()],
            |pixels, position, (r, g, b, a): (f32, f32, f32, f32)| {
                pixels[position.y() * 32 + position.x()] = [r, g, b, a]
            },
        )
        .unwrap();
        let expected = crate::color::ap1_to_ap0() * pixel(0, 0).unwrap();
        for c in 0..3 {
            assert_eq!(
                half_image.layer_data.channel_data.pixels[0][c],
                half::f16::from_f64(expected[c]).to_f32()
            );
        }
        editor.source = true;
        editor.source_exposure = 2.;
        editor.viewport.zoom = Some(1.5);
        let view = format!("{:?}", editor.viewport);
        let mut disabled = editor.document.clone();
        disabled.project.flat_field.as_mut().unwrap().enabled = false;
        editor.start_document(DocumentWork::Correction(disabled, Some(selected)));
        wait(&mut editor);
        assert!(editor.correction.is_none());
        assert!(editor.source);
        assert_eq!(editor.source_exposure, 2.);
        assert_eq!(format!("{:?}", editor.viewport), view);
        assert_eq!((editor.dmin, editor.dmax), rgb);
    });
}
fn wait(editor: &mut Editor) {
    let start = std::time::Instant::now();
    while editor.busy {
        editor.receive();
        assert!(
            start.elapsed() < Duration::from_secs(120),
            "{}",
            editor.status
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
