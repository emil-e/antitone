use super::*;
pub(super) fn fixture() -> Document {
    let mut document = Document::default();
    for index in 0..40 {
        document
            .register(
                Source {
                    id: SourceId(0),
                    path: format!("photo-{index}.ARW").into(),
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
pub(super) fn key(document: &Document, id: SourceId) -> ImageKey {
    ImageKey {
        source: SourceKey::new(
            document.project.source(id).unwrap(),
            document.project.development.as_ref().unwrap(),
            FileKey::new(&document.source_path(id).unwrap()),
        ),
        target: OutputTarget::DisplayP3,
        field: document
            .project
            .active_flat_field()
            .map(|f| FieldKey(f.map.clone())),
    }
}
#[test]
fn transient_reference_with_the_same_development_cannot_seed_the_working_photo() {
    let (mut editor, _, _) = super::super::tests::fixture();
    super::super::tests::add_ready_fixture(&mut editor);
    let ready = editor.ready.as_mut().unwrap();
    ready.record.path = "different-reference.ARW".into();
    ready.thumbnail = Some((
        FileKey::new(&ready.record.path),
        Arc::new(Proxy {
            width: 1,
            height: 1,
            pixels: vec![[0; 4]],
            matrix: glam::DMat3::IDENTITY,
        }),
    ));
    editor.seed_thumbnail();
    assert!(editor.thumbnails.sources.lock().unwrap().entries.is_empty());
}

#[test]
fn file_stats_and_resolved_locators_keep_cache_revisions_separate() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("photo.ARW");
    std::fs::write(&path, b"original").unwrap();
    let original = FileKey::new(&path);
    assert!(original.revision.is_some());
    std::fs::create_dir(directory.path().join("other-card")).unwrap();
    let other = directory.path().join("other-card/photo.ARW");
    std::fs::copy(&path, &other).unwrap();
    assert_ne!(FileKey::new(&other), original);
    #[cfg(unix)]
    {
        let alias = directory.path().join("alias.ARW");
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        assert_eq!(FileKey::new(&alias), original);
    }
    #[cfg(unix)]
    {
        std::fs::write(&path, b"modified").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(original.revision.as_ref().unwrap().modified),
            )
            .unwrap();
        let edited = FileKey::new(&path);
        assert_eq!(
            edited.revision.as_ref().unwrap().bytes,
            original.revision.as_ref().unwrap().bytes
        );
        assert_eq!(
            edited.revision.as_ref().unwrap().modified,
            original.revision.as_ref().unwrap().modified
        );
        assert_ne!(edited, original);
    }
    std::fs::write(&path, b"intentionally edited source").unwrap();
    assert_ne!(FileKey::new(&path), original);
    let edited = FileKey::new(&path);
    std::fs::remove_file(&path).unwrap();
    std::fs::copy(&other, &path).unwrap();
    assert_ne!(FileKey::new(&path), edited);
    std::fs::remove_file(&path).unwrap();
    assert!(FileKey::new(&path).revision.is_none());
}

#[test]
fn queued_thumbnail_results_do_not_install_after_file_stats_change() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("photo.ARW");
    std::fs::write(&path, b"before").unwrap();
    let mut document = fixture();
    let id = document.project.order[0];
    document.project.source_mut(id).unwrap().path = path.clone();
    let (commands, receiver) = mpsc::channel();
    let (send, messages) = mpsc::channel();
    let mut thumbnails = Thumbnails {
        commands: Some(commands),
        messages: Some(messages),
        ..Default::default()
    };
    thumbnails.request(
        key(&document, id),
        path.clone(),
        document.project.source(id).unwrap().clone(),
        None,
    );
    let job = receiver.try_recv().unwrap();
    std::fs::write(&path, b"after a deliberate edit").unwrap();
    send.send(Outcome {
        ticket: job.ticket,
        key: job.key,
        image: Err("old outcome".into()),
    })
    .unwrap();
    thumbnails.receive(&document);
    assert!(thumbnails.cache.entries.is_empty());
    assert!(thumbnails.pending.is_none());
}

#[test]
fn cache_is_bounded_lru_and_replacement_does_not_grow_it() {
    const CAPACITY: usize = 16;
    let mut cache = Cache::new(CAPACITY, |_| 1);
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
fn source_cache_key_excludes_grading_but_includes_locator_revision_matrix_wb_and_id() {
    let mut document = fixture();
    let id = document.project.order[0];
    let original = key(&document, id);
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
    document.project.source_mut(id).unwrap().path = "moved.ARW".into();
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
    editor.photo = PhotoSettings {
        exposure: 1.25,
        contrast: 1.2,
        pivot: 0.25,
        contrast_colour_mix: 0.75,
        saturation: 1.3,
        saturation_density: 0.25,
        ..PhotoSettings::default()
    };
    let adjusted = editor.thumbnail_parameters(&source).unwrap();
    let expected = ProcessingParameters::positive(
        editor.calibration().unwrap(),
        editor.roll,
        editor.photo,
        glam::DMat3::IDENTITY,
    )
    .unwrap();
    assert_eq!(bytemuck::bytes_of(&adjusted), bytemuck::bytes_of(&expected));
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

#[test]
fn critical_pressure_cancels_work_once_and_drops_optional_images() {
    let document = fixture();
    let id = document.project.order[0];
    let (commands, receiver) = mpsc::channel();
    let mut thumbnails = Thumbnails {
        commands: Some(commands),
        ..Default::default()
    };
    thumbnails.apply_budgets(memory::CacheBudgets::for_memory(
        48 << 30,
        memory::Pressure::Normal,
    ));
    thumbnails
        .cache
        .insert(key(&document, id), Err("cached error".into()));
    thumbnails.request(
        key(&document, document.project.order[1]),
        document.source_path(id).unwrap(),
        document.project.source(id).unwrap().clone(),
        None,
    );
    receiver.try_recv().unwrap();
    let generation = thumbnails
        .generation
        .load(std::sync::atomic::Ordering::Relaxed);
    let critical = memory::CacheBudgets::for_memory(48 << 30, memory::Pressure::Critical);
    thumbnails.apply_budgets(critical);
    assert!(thumbnails.cache.entries.is_empty());
    assert_eq!(
        thumbnails
            .generation
            .load(std::sync::atomic::Ordering::Relaxed),
        generation + 1
    );
    thumbnails.apply_budgets(critical);
    assert_eq!(
        thumbnails
            .generation
            .load(std::sync::atomic::Ordering::Relaxed),
        generation + 1
    );
}

#[test]
fn visible_jobs_do_not_wait_for_refinement_or_grading_idle() {
    let (commands, _) = mpsc::channel();
    let (_, messages) = mpsc::channel();
    let mut editor = Editor::state(commands, messages, Arc::default());
    editor.document = fixture();
    let first = editor.document.project.order[0];
    let last = *editor.document.project.order.last().unwrap();
    for id in editor
        .document
        .project
        .order
        .clone()
        .into_iter()
        .filter(|id| *id != first && *id != last)
    {
        editor
            .thumbnails
            .cache
            .insert(key(&editor.document, id), Err("already visited".into()));
    }
    let (commands, jobs) = mpsc::channel();
    let (completed, outcomes) = mpsc::channel();
    editor.thumbnails.commands = Some(commands);
    editor.thumbnails.messages = Some(outcomes);
    editor.refining = Some((
        0,
        crate::refinement::ViewRequest {
            area: [0, 0, 1, 1].into(),
            output: [1, 1],
            frame: None,
        },
    ));
    editor.last_edit = std::time::Instant::now();
    let context = egui::Context::default();
    let frame = |editor: &mut Editor| {
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100., 800.),
                )),
                ..Default::default()
            },
            |ui| editor.filmstrip(ui),
        );
        output.textures_delta.clear();
    };
    frame(&mut editor);
    let job = jobs
        .try_recv()
        .expect("missing visible cell must not wait for settled refinement");
    assert_eq!(job.key.source.id, first);
    frame(&mut editor);
    assert!(
        jobs.try_recv().is_err(),
        "only one outstanding thumbnail job"
    );
    completed
        .send(Outcome {
            ticket: job.ticket,
            key: job.key,
            image: Err("fixture completed".into()),
        })
        .unwrap();
    editor.thumbnails.receive(&editor.document);
    frame(&mut editor);
    assert!(
        jobs.try_recv().is_err(),
        "offscreen work still requires idle"
    );
}

#[test]
fn visible_jobs_remain_suspended_for_foreground_and_reference_work() {
    for suspension in ["busy", "pick", "document"] {
        let (commands, _) = mpsc::channel();
        let (_, messages) = mpsc::channel();
        let mut editor = Editor::state(commands, messages, Arc::default());
        editor.document = fixture();
        let (commands, jobs) = mpsc::channel();
        editor.thumbnails.commands = Some(commands);
        match suspension {
            "busy" => editor.busy = true,
            "pick" => editor.pick = Some(super::super::Pick::Dmin),
            "document" => editor.document_pending = Some(0),
            _ => unreachable!(),
        }
        let context = egui::Context::default();
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100., 800.),
                )),
                ..Default::default()
            },
            |ui| editor.filmstrip(ui),
        );
        output.textures_delta.clear();
        assert!(
            jobs.try_recv().is_err(),
            "{suspension} must block visible admission"
        );
    }
}

#[test]
fn worker_completion_requests_immediate_repaint() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let renderers = Arc::new(crate::renderer::RendererCache::default());
        let mut thumbnails = Thumbnails::with_disk(device, queue, renderers, None);
        let context = egui::Context::default();
        for _ in 0..3 {
            let mut output = context.run_ui(Default::default(), |_| {});
            output.textures_delta.clear();
        }
        context.request_repaint_after(Duration::from_millis(50));
        let (send, wake) = mpsc::channel();
        context.set_request_repaint_callback(move |info| {
            if info.delay == Duration::ZERO {
                let _ = send.send(());
            }
        });
        thumbnails.repaint.get_or_init(|| context);
        let document = fixture();
        let id = document.project.order[0];
        thumbnails.request(
            key(&document, id),
            document.source_path(id).unwrap(),
            document.project.source(id).unwrap().clone(),
            None,
        );
        wake.recv_timeout(Duration::from_secs(10))
            .expect("completion must wake UI immediately, not await the polling timer");
        thumbnails.receive(&document);
        assert!(thumbnails.pending.is_none());
        assert!(matches!(
            thumbnails.cache.get(&key(&document, id)),
            Some(Err(_))
        ));
    });
}

#[test]
fn idle_roll_prefetch_reaches_unshown_photos_after_visible_work() {
    let (commands, _) = mpsc::channel();
    let (_, messages) = mpsc::channel();
    let mut editor = Editor::state(
        commands,
        messages,
        Arc::new(std::sync::atomic::AtomicU64::new(0)),
    );
    editor.document = fixture();
    let last = *editor.document.project.order.last().unwrap();
    for id in editor
        .document
        .project
        .order
        .clone()
        .into_iter()
        .filter(|id| *id != last)
    {
        editor
            .thumbnails
            .cache
            .insert(key(&editor.document, id), Err("already visited".into()));
    }
    let (commands, jobs) = mpsc::channel();
    let (completed, outcomes) = mpsc::channel();
    editor.thumbnails.commands = Some(commands);
    editor.thumbnails.messages = Some(outcomes);
    editor.refinement_error = Some(0);
    editor.last_edit = std::time::Instant::now() - Duration::from_secs(1);
    let context = egui::Context::default();
    let input = || egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1100., 800.),
        )),
        ..Default::default()
    };
    let mut output = context.run_ui(input(), |ui| editor.filmstrip(ui));
    output.textures_delta.clear();
    let job = jobs.try_recv().unwrap();
    assert_eq!(
        job.key.source.id, last,
        "should prefetch even the unshown far end of the roll"
    );
    editor.last_edit = std::time::Instant::now();
    let generation = editor
        .thumbnails
        .generation
        .load(std::sync::atomic::Ordering::Relaxed);
    let mut output = context.run_ui(input(), |ui| editor.filmstrip(ui));
    output.textures_delta.clear();
    assert_eq!(
        editor
            .thumbnails
            .generation
            .load(std::sync::atomic::Ordering::Relaxed),
        generation + 1,
        "resumed foreground work cancels far idle prefetch"
    );
    assert!(jobs.try_recv().is_err());
    completed
        .send(Outcome {
            ticket: job.ticket,
            key: job.key,
            image: Ok(None),
        })
        .unwrap();
    editor.thumbnails.receive(&editor.document);
    editor.last_edit = std::time::Instant::now() - Duration::from_secs(1);
    let mut output = context.run_ui(input(), |ui| editor.filmstrip(ui));
    output.textures_delta.clear();
    assert_eq!(jobs.try_recv().unwrap().key.source.id, last);
    let first = editor.document.project.order[0];
    editor
        .thumbnails
        .cache
        .retain(|key, _| key.source.id != first);
    let before = editor
        .thumbnails
        .generation
        .load(std::sync::atomic::Ordering::Relaxed);
    let mut output = context.run_ui(input(), |ui| editor.filmstrip(ui));
    output.textures_delta.clear();
    assert_eq!(
        editor
            .thumbnails
            .generation
            .load(std::sync::atomic::Ordering::Relaxed),
        before + 1,
        "missing visible work preempts idle far prefetch"
    );
    assert!(
        jobs.try_recv().is_err(),
        "cancellation must not create an unbounded backlog"
    );
}

#[test]
fn crop_draft_cancels_offscreen_prefetch_and_suspends_idle_admission() {
    let (commands, _) = mpsc::channel();
    let (_, messages) = mpsc::channel();
    let mut editor = Editor::state(
        commands,
        messages,
        Arc::new(std::sync::atomic::AtomicU64::new(0)),
    );
    editor.document = fixture();
    editor.floor = editor.document.project.input_floor.unwrap();
    super::super::tests::add_ready_fixture(&mut editor);
    let last = *editor.document.project.order.last().unwrap();
    let image_key = key(&editor.document, last);
    let (commands, jobs) = mpsc::channel();
    let (completed, outcomes) = mpsc::channel();
    editor.thumbnails.commands = Some(commands);
    editor.thumbnails.messages = Some(outcomes);
    editor.thumbnails.request(
        image_key.clone(),
        editor.document.source_path(last).unwrap(),
        editor.document.project.source(last).unwrap().clone(),
        None,
    );
    let job = jobs.try_recv().unwrap();
    editor.begin_crop();
    assert!(editor.crop_edit.is_some());
    editor.refinement_error = Some(editor.generation.load(std::sync::atomic::Ordering::Relaxed));
    editor.last_edit = std::time::Instant::now() - Duration::from_secs(1);
    let before = editor
        .thumbnails
        .generation
        .load(std::sync::atomic::Ordering::Relaxed);
    let context = egui::Context::default();
    let frame = |editor: &mut Editor| {
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100., 800.),
                )),
                ..Default::default()
            },
            |ui| editor.filmstrip(ui),
        );
        output.textures_delta.clear();
    };
    frame(&mut editor);
    completed
        .send(Outcome {
            ticket: job.ticket,
            key: image_key,
            image: Ok(None),
        })
        .unwrap();
    // Release the cancelled request before checking that the draft admits no replacement.
    editor.thumbnails.receive(&editor.document);
    frame(&mut editor);
    assert_eq!(
        editor
            .thumbnails
            .generation
            .load(std::sync::atomic::Ordering::Relaxed),
        before + 1
    );
    assert!(editor.thumbnails.pending.is_none());
    assert!(jobs.try_recv().is_err());
}

#[test]
fn warm_normal_roll_survives_rapid_forward_and_reverse_scrolling() {
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
        let (commands, _) = mpsc::channel();
        let (_, messages) = mpsc::channel();
        let mut editor = Editor::state(
            commands,
            messages,
            Arc::new(std::sync::atomic::AtomicU64::new(0)),
        );
        editor.document = fixture();
        editor.document.project.order.truncate(36);
        for id in editor.document.project.order.clone() {
            let image = Arc::new(
                GpuImage::new_shared(
                    &renderer,
                    &device,
                    8,
                    8,
                    &[[0; 4]; 64],
                    glam::DMat3::IDENTITY,
                    OutputTarget::DisplayP3,
                )
                .await
                .unwrap(),
            );
            editor
                .thumbnails
                .cache
                .insert(key(&editor.document, id), Ok(image));
        }
        let context = egui::Context::default();
        let mut names = std::collections::HashSet::new();
        for delta in [0., -4000., -4000., 4000., 4000., -4000., 4000.] {
            for frame in 0..12 {
                let mut output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1100., 800.),
                        )),
                        events: if frame == 0 {
                            vec![
                                egui::Event::PointerMoved(egui::pos2(550., 750.)),
                                egui::Event::MouseWheel {
                                    unit: egui::MouseWheelUnit::Point,
                                    phase: egui::TouchPhase::Move,
                                    delta: egui::vec2(delta, 0.),
                                    modifiers: egui::Modifiers::NONE,
                                },
                            ]
                        } else {
                            vec![]
                        },
                        ..Default::default()
                    },
                    |ui| editor.filmstrip(ui),
                );
                output.textures_delta.clear();
                for shape in output.shapes {
                    if let egui::Shape::Text(text) = shape.shape {
                        assert_ne!(text.galley.job.text, "…", "warm thumbnail was evicted");
                        if text.galley.job.text.starts_with("photo-") {
                            names.insert(text.galley.job.text.clone());
                        }
                    }
                }
            }
        }
        assert!(names.contains("photo-0.ARW") && names.contains("photo-35.ARW"));
        assert_eq!(editor.thumbnails.cache.entries.len(), 36);
    });
}

#[test]
fn critical_pressure_releases_the_seed_when_the_working_source_is_gone() {
    let document = fixture();
    let source = document.project.source(document.project.order[0]).unwrap();
    let mut thumbnails = Thumbnails::default();
    thumbnails.sources.lock().unwrap().set_budget(0);
    thumbnails.seed(
        source,
        document.project.development.as_ref().unwrap(),
        FileKey::new(&document.source_path(source.id).unwrap()),
        Arc::new(Proxy {
            width: 1,
            height: 1,
            pixels: vec![[0; 4]],
            matrix: glam::DMat3::IDENTITY,
        }),
    );
    assert_eq!(thumbnails.sources.lock().unwrap().entries.len(), 1);
    thumbnails.release_selected();
    assert!(thumbnails.sources.lock().unwrap().entries.is_empty());
}

#[test]
fn obsolete_scroll_work_is_cancelled_once_without_dropping_valid_source_seeds() {
    let document = fixture();
    let id = document.project.order[0];
    let (commands, receiver) = mpsc::channel();
    let mut thumbnails = Thumbnails {
        commands: Some(commands),
        ..Default::default()
    };
    let source = document.project.source(id).unwrap();
    let proxy = Arc::new(Proxy {
        width: 1,
        height: 1,
        pixels: vec![[1; 4]],
        matrix: glam::DMat3::IDENTITY,
    });
    thumbnails.seed(
        source,
        document.project.development.as_ref().unwrap(),
        FileKey::new(&document.source_path(source.id).unwrap()),
        proxy,
    );
    let old = key(&document, id);
    thumbnails.request(old.clone(), source.path.clone(), source.clone(), None);
    receiver.try_recv().unwrap();
    thumbnails.cancel_offscreen(&[old]);
    let before = thumbnails
        .generation
        .load(std::sync::atomic::Ordering::Relaxed);
    thumbnails.cancel_offscreen(&[]);
    assert_eq!(
        thumbnails
            .generation
            .load(std::sync::atomic::Ordering::Relaxed),
        before + 1
    );
    thumbnails.cancel_offscreen(&[]);
    assert_eq!(
        thumbnails
            .generation
            .load(std::sync::atomic::Ordering::Relaxed),
        before + 1
    );
    assert!(thumbnails.pending.is_some());
    assert_eq!(thumbnails.sources.lock().unwrap().entries.len(), 1);
}

#[test]
fn disk_worker_reuses_checksummed_pixels_and_changed_stats_miss_instead_of_rejecting_content() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("not-a-raw.ARW");
        std::fs::write(&path, b"These bytes cannot be developed by LibRaw").unwrap();
        let mut document = Document::default();
        let mut source = fixture().project.sources.remove(0);
        source.path = path.clone();
        source.development.dimensions = [2, 1];
        source.development.active_area = [0, 0, 2, 1];
        let development = Development::new(crate::raw::version(), [1.; 4]);
        let id = document.register(source, development.clone()).unwrap();
        let source = document.project.source(id).unwrap().clone();
        let disk = disk::DiskCache::at(directory.path().join("cache"));
        disk.store(
            &SourceKey::new(&source, &development, FileKey::new(&path)),
            &Proxy {
                width: 2,
                height: 1,
                pixels: vec![[1, 2, 3, 0]; 2],
                matrix: glam::DMat3::IDENTITY,
            },
        )
        .unwrap();
        let renderers = Arc::new(crate::renderer::RendererCache::default());
        let mut thumbnails =
            Thumbnails::with_disk(device.clone(), queue.clone(), renderers.clone(), Some(disk));
        thumbnails.request(key(&document, id), path.clone(), source.clone(), None);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while thumbnails.pending.is_some() {
            thumbnails.receive(&document);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            thumbnails.ready_count(&document, OutputTarget::DisplayP3),
            1
        );
        std::fs::write(&path, b"changed").unwrap();
        let mut restarted = Thumbnails::with_disk(
            device,
            queue,
            renderers,
            Some(disk::DiskCache::at(directory.path().join("cache"))),
        );
        restarted.request(key(&document, id), path, source, None);
        while restarted.pending.is_some() {
            restarted.receive(&document);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            matches!(restarted.cache.get(&key(&document, id)).unwrap(), Err(error) if !error.contains("content changed"))
        );
        assert!(
            disk::DiskCache::at(directory.path().join("cache"))
                .load(&key(&document, id).source)
                .unwrap()
                .is_none()
        );
    });
}
