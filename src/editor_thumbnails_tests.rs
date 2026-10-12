use super::*;
fn fixture() -> Document {
    let mut document = Document::default();
    for index in 0..40 {
        document
            .register(
                Source {
                    id: SourceId(0),
                    path: format!("photo-{index}.ARW").into(),
                    fingerprint: Fingerprint {
                        bytes: index + 1,
                        blake3: "a".repeat(64),
                    },
                    development: SourceDevelopment {
                        dimensions: [900, 600],
                        active_area: [0, 0, 900, 600],
                        camera_model: "test-camera".into(),
                        camera_to_ap1: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                        input_floor: crate::processing::CAMERA_INPUT_FLOOR,
                    },
                    reference_only: false,
                    photo: PhotoSettings::default(),
                    orientation: Default::default(),
                    crop: None,
                },
                Development::new(crate::raw::version(), [2., 1., 1.5, 1.]),
            )
            .unwrap();
    }
    document
}
fn key(document: &Document, id: SourceId) -> ImageKey {
    ImageKey {
        source: SourceKey::new(
            document.project.source(id).unwrap(),
            document.project.development.as_ref().unwrap(),
        ),
        target: OutputTarget::DisplayP3,
        field: document
            .project
            .active_flat_field()
            .map(|f| FieldKey(f.map.clone())),
    }
}
#[test]
fn cache_is_bounded_lru_and_replacement_does_not_grow_it() {
    let mut cache = Cache::default();
    for index in 0..CAPACITY {
        cache.insert(index, index);
    }
    assert_eq!(cache.get(&0), Some(&0));
    cache.insert(CAPACITY, CAPACITY);
    assert!(cache.get(&1).is_none());
    assert_eq!(cache.get(&0), Some(&0));
    cache.insert(0, 42);
    assert_eq!(cache.get(&0), Some(&42));
    assert_eq!(cache.entries.len(), CAPACITY);
}
#[test]
fn source_identity_excludes_locator_photo_and_roll_edits_but_includes_bytes_matrix_wb_and_id() {
    let mut document = fixture();
    let id = document.project.order[0];
    let original = key(&document, id);
    document.project.source_mut(id).unwrap().path = "moved.ARW".into();
    document.project.source_mut(id).unwrap().photo.exposure = 7.12345;
    document.project.roll.exposure = 1.5;
    document.project.orientation.horizontal = true;
    document.project.orientation.turn_clockwise();
    document
        .project
        .source_mut(id)
        .unwrap()
        .orientation
        .turn_clockwise();
    document.project.source_mut(id).unwrap().crop = Some(Crop::from_region(Rect {
        x: 30,
        y: 20,
        width: 401,
        height: 203,
    }));
    // Composition is applied on paint, so source/processed textures remain reusable.
    assert_eq!(key(&document, id), original);
    document.project.source_mut(id).unwrap().fingerprint.bytes += 1;
    assert_ne!(key(&document, id), original);
    let mut candidate = original.clone();
    candidate.source.source.camera_to_ap1[0][0] += 0.1;
    assert_ne!(candidate, original);
    candidate = original.clone();
    candidate.source.development.fixed_wb[0] += 0.1;
    assert_ne!(candidate, original);
    candidate = original.clone();
    candidate.source.id = SourceId(42);
    assert_ne!(candidate, original);
    candidate = original.clone();
    candidate.target = OutputTarget::Srgb;
    assert_ne!(candidate, original);
}
#[test]
fn correction_changes_image_identity_without_invalidating_unconverted_raw_proxies() {
    let mut document = fixture();
    let id = document.project.order[0];
    let plain = key(&document, id);
    let source = document.project.source(id).unwrap();
    let a = source.development.active_area;
    let map = Arc::new(
        crate::flat_field::FlatField::from_nodes(a, [2, 2], vec![DVec3::splat(1.1); 4]).unwrap(),
    );
    document.project.flat_field = Some(crate::project::FlatFieldSettings::new(
        source,
        map,
        document.project.development.as_ref().unwrap(),
    ));
    let corrected = key(&document, id);
    assert_ne!(corrected, plain);
    assert_eq!(corrected.source, plain.source);
    let mut recreated = corrected.clone();
    recreated.field = Some(FieldKey(Arc::new(
        (*corrected.field.as_ref().unwrap().0).clone(),
    )));
    assert_eq!(corrected, recreated);
    document.project.flat_field.as_mut().unwrap().enabled = false;
    assert_eq!(key(&document, id), plain);
    document.project.flat_field = None;
    assert_eq!(key(&document, id), plain);
}
#[test]
fn one_outstanding_job_is_kept_until_cancelled_completion_and_stale_results_do_not_install() {
    let document = fixture();
    let id = document.project.order[0];
    let (commands, receiver) = mpsc::channel();
    let (send, messages) = mpsc::channel();
    let mut thumbnails = Thumbnails {
        commands: Some(commands),
        messages: Some(messages),
        ..Default::default()
    };
    let image_key = key(&document, id);
    thumbnails.request(
        image_key.clone(),
        document.source_path(id).unwrap(),
        document.project.source(id).unwrap().clone(),
        None,
    );
    let job = receiver.try_recv().unwrap();
    thumbnails.cancel();
    thumbnails.request(
        image_key.clone(),
        document.source_path(id).unwrap(),
        document.project.source(id).unwrap().clone(),
        None,
    );
    assert!(receiver.try_recv().is_err());
    assert!(thumbnails.pending.is_some());
    send.send(Outcome {
        ticket: job.ticket,
        key: job.key,
        image: Err("stale failure".into()),
    })
    .unwrap();
    thumbnails.receive(&document);
    assert!(thumbnails.pending.is_none());
    assert!(thumbnails.cache.entries.is_empty());
    thumbnails.request(
        image_key,
        document.source_path(id).unwrap(),
        document.project.source(id).unwrap().clone(),
        None,
    );
    assert!(receiver.try_recv().is_ok());
}
#[test]
fn changed_document_identity_rejects_result_and_worker_disconnection_clears_pending() {
    let mut document = fixture();
    let id = document.project.order[0];
    let (commands, receiver) = mpsc::channel();
    let (send, messages) = mpsc::channel();
    let mut thumbnails = Thumbnails {
        commands: Some(commands),
        messages: Some(messages),
        ..Default::default()
    };
    thumbnails.request(
        key(&document, id),
        document.source_path(id).unwrap(),
        document.project.source(id).unwrap().clone(),
        None,
    );
    let job = receiver.try_recv().unwrap();
    document.project.development.as_mut().unwrap().fixed_wb[0] += 0.1;
    send.send(Outcome {
        ticket: job.ticket,
        key: job.key,
        image: Err("old WB".into()),
    })
    .unwrap();
    thumbnails.receive(&document);
    assert!(thumbnails.cache.entries.is_empty());
    thumbnails.request(
        key(&document, id),
        document.source_path(id).unwrap(),
        document.project.source(id).unwrap().clone(),
        None,
    );
    receiver.try_recv().unwrap();
    drop(send);
    thumbnails.receive(&document);
    assert!(thumbnails.pending.is_none());
    assert!(thumbnails.commands.is_none());
    assert!(thumbnails.cache.get(&key(&document, id)).unwrap().is_err());
}
#[test]
fn filmstrip_resize_and_selection_preserve_project_state_and_loading_placeholders() {
    let (commands, receiver) = mpsc::channel();
    let (_send, messages) = mpsc::channel();
    let mut editor = Editor::state(
        commands,
        messages,
        Arc::new(std::sync::atomic::AtomicU64::new(0)),
    );
    editor.document = fixture();
    editor.floor = editor.document.project.input_floor.unwrap();
    let before = editor.document.project.clone();
    let context = egui::Context::default();
    let mut run = |events| {
        let mut source_positions = Vec::new();
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100., 800.),
                )),
                events,
                ..Default::default()
            },
            |ui| editor.filmstrip(ui),
        );
        output.textures_delta.clear();
        for shape in output.shapes {
            if let egui::Shape::Text(text) = shape.shape
                && text.galley.job.text.starts_with("photo-")
            {
                source_positions.push(text.pos);
            }
        }
        source_positions
    };
    let positions = run(vec![]);
    assert!(!positions.is_empty());
    assert!(positions.len() < 40);
    let initial =
        egui::containers::panel::PanelState::load(&context, egui::Id::new("thumbnail-filmstrip"))
            .unwrap()
            .outer_rect;
    assert!(
        (initial.height() - 140.).abs() < 1.,
        "initial strip height: {}",
        initial.height()
    );
    let edge = initial.center_top();
    run(vec![
        egui::Event::PointerMoved(edge),
        egui::Event::PointerButton {
            pos: edge,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    run(vec![egui::Event::PointerMoved(edge - egui::vec2(0., 50.))]);
    run(vec![egui::Event::PointerButton {
        pos: edge - egui::vec2(0., 50.),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    let resized =
        egui::containers::panel::PanelState::load(&context, egui::Id::new("thumbnail-filmstrip"))
            .unwrap()
            .outer_rect;
    assert!(resized.height() > initial.height() + 40.);
    for _ in 0..20 {
        run(vec![]);
        let idle = egui::containers::panel::PanelState::load(
            &context,
            egui::Id::new("thumbnail-filmstrip"),
        )
        .unwrap()
        .outer_rect;
        assert!(
            (idle.height() - resized.height()).abs() < 1.,
            "filmstrip shrank after release: {} -> {}",
            resized.height(),
            idle.height()
        );
    }
    let positions = run(vec![]);
    let point = positions[0] + egui::vec2(10., -20.);
    for pressed in [true, false] {
        run(vec![
            egui::Event::PointerMoved(point),
            egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
    }
    drop(run);
    assert!(matches!(receiver.try_recv().unwrap(), Command::Load(..)));
    assert!((editor.preferences.filmstrip_height - resized.height()).abs() < 1.);
    assert_eq!(editor.document.project, before);
}
#[test]
fn restored_filmstrip_height_survives_window_constraints_and_a_fresh_context() {
    let (mut editor, _send, _receiver) = crate::editor::tests::fixture();
    editor.preferences.filmstrip_height = 210.;
    let before = editor.document.project.clone();
    for context in [egui::Context::default(), egui::Context::default()] {
        for height in [800., 300., 20., 800.] {
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100., height),
                    )),
                    ..Default::default()
                },
                |ui| editor.filmstrip(ui),
            );
            output.textures_delta.clear();
            let panel = egui::containers::panel::PanelState::load(
                &context,
                egui::Id::new("thumbnail-filmstrip"),
            )
            .unwrap();
            let expected = if height == 300. {
                105.
            } else if height == 20. {
                20.
            } else {
                210.
            };
            assert!((panel.outer_rect.height() - expected).abs() < 1.);
            assert_eq!(editor.preferences.filmstrip_height, 210.);
        }
    }
    assert_eq!(editor.document.project, before);
}

#[test]
fn thumbnail_parameters_follow_actual_roll_and_each_photo_without_inspection_gain_or_source_redecode()
 {
    let (commands, _receiver) = mpsc::channel();
    let (_send, messages) = mpsc::channel();
    let mut editor = Editor::state(
        commands,
        messages,
        Arc::new(std::sync::atomic::AtomicU64::new(0)),
    );
    editor.document = fixture();
    let id = editor.document.project.order[0];
    let other = editor.document.project.order[1];
    editor.current_source = Some(id);
    editor.source_exposure = 3.;
    let source = editor.document.project.source(id).unwrap().clone();
    assert_eq!(
        bytemuck::bytes_of(&editor.thumbnail_parameters(&source).unwrap()),
        bytemuck::bytes_of(&ProcessingParameters::source())
    );
    editor.dmin = Some(DVec3::new(0.2, 0.15, 0.1));
    editor.dmax = Some(DVec3::new(0.008, 0.005, 0.002));
    editor.floor = crate::processing::CAMERA_INPUT_FLOOR;
    let first = editor.thumbnail_parameters(&source).unwrap();
    let expected = ProcessingParameters::positive(
        editor.calibration().unwrap(),
        editor.roll,
        editor.photo,
        glam::DMat3::IDENTITY,
    )
    .unwrap();
    assert_eq!(bytemuck::bytes_of(&first), bytemuck::bytes_of(&expected));
    let unchanged_key = key(&editor.document, id);
    editor.photo.exposure = 1.25;
    let adjusted = editor.thumbnail_parameters(&source).unwrap();
    assert_ne!(bytemuck::bytes_of(&adjusted), bytemuck::bytes_of(&first));
    assert_eq!(
        bytemuck::bytes_of(
            &editor
                .thumbnail_parameters(editor.document.project.source(other).unwrap())
                .unwrap()
        ),
        bytemuck::bytes_of(&first)
    );
    editor.roll.exposure = 0.5;
    assert_ne!(
        bytemuck::bytes_of(
            &editor
                .thumbnail_parameters(editor.document.project.source(other).unwrap())
                .unwrap()
        ),
        bytemuck::bytes_of(&first)
    );
    assert_eq!(key(&editor.document, id), unchanged_key);
    editor
        .document
        .project
        .source_mut(other)
        .unwrap()
        .photo
        .exposure = -1.25;
    let expected = ProcessingParameters::positive(
        editor.calibration().unwrap(),
        editor.roll,
        editor.document.project.source(other).unwrap().photo,
        glam::DMat3::IDENTITY,
    )
    .unwrap();
    assert_eq!(
        bytemuck::bytes_of(
            &editor
                .thumbnail_parameters(editor.document.project.source(other).unwrap())
                .unwrap()
        ),
        bytemuck::bytes_of(&expected)
    );
}
