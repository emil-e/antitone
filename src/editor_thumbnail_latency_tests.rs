use super::*;

#[test]
#[ignore = "read-only local RAW/Metal import to first/all-visible thumbnail readiness"]
fn visible_thumbnail_import_latency() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        println!(
            "visible_thumbnail_probe boundary=ui_cache_ready_not_presentation filesystem_cache=uncontrolled window=1100x800 cold_then_disk roll_sizes=4,36"
        );
        for roll in [4, 36] {
            let directory = tempfile::tempdir().unwrap();
            let cache = directory.path().join("thumbnails");
            let paths: Vec<_> = (4378..4378 + roll)
                .map(|number| PathBuf::from(format!("samples/DSC0{number}.ARW")))
                .collect();
            for state in ["cold", "disk"] {
                let mut editor = Editor::new(&device, &queue, None);
                let renderers = editor.thumbnails.gpu.as_ref().unwrap().2.clone();
                editor.thumbnails = Thumbnails::with_disk(
                    device.clone(),
                    queue.clone(),
                    renderers,
                    Some(disk::DiskCache::at(cache.clone())),
                );
                let context = egui::Context::default();
                let input = || egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100., 800.),
                    )),
                    ..Default::default()
                };
                let frame = |editor: &mut Editor| {
                    let mut output = context.run_ui(input(), |ui| host::App::ui(editor, ui));
                    output.textures_delta.clear();
                    output
                        .shapes
                        .into_iter()
                        .filter_map(|shape| {
                            if let egui::Shape::Text(text) = shape.shape {
                                Some(text.galley.job.text.clone())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                };
                for _ in 0..3 {
                    frame(&mut editor);
                }
                let started = std::time::Instant::now();
                editor.add_raws(paths.clone());
                let mut prepared = None;
                let mut first = None;
                let mut camera_first = None;
                let mut navigable_all = None;
                let mut visible = Vec::new();
                let mut first_refining = false;
                let mut first_settled = false;
                let deadline = started + Duration::from_secs(30);
                loop {
                    let labels = frame(&mut editor);
                    if !editor.busy && prepared.is_none() {
                        prepared = Some(started.elapsed());
                    }
                    if visible.is_empty() && !editor.document.project.order.is_empty() {
                        visible = editor
                            .document
                            .project
                            .order
                            .iter()
                            .filter_map(|id| {
                                let path = editor.document.source_path(*id).unwrap();
                                let name = path.file_name().unwrap().to_string_lossy();
                                labels.iter().any(|label| label == &name).then(|| ImageKey {
                                    source: SourceKey::new(
                                        editor.document.project.source(*id).unwrap(),
                                        editor.document.project.development.as_ref().unwrap(),
                                        FileKey::new(&path),
                                    ),
                                    target: editor.target,
                                    field: None,
                                })
                            })
                            .collect();
                        assert!(!visible.is_empty());
                    }
                    let ready = visible
                        .iter()
                        .filter(|key| matches!(editor.thumbnails.cache.get(key), Some(Ok(_))))
                        .count();
                    let camera_count = visible
                        .iter()
                        .filter(|key| editor.thumbnails.embedded.get(&key.source).is_some())
                        .count();
                    let navigable = visible
                        .iter()
                        .filter(|key| {
                            matches!(editor.thumbnails.cache.get(key), Some(Ok(_)))
                                || editor.thumbnails.embedded.get(&key.source).is_some()
                        })
                        .count();
                    if camera_count > 0 && camera_first.is_none() {
                        camera_first = Some(started.elapsed());
                    }
                    if !visible.is_empty() && navigable == visible.len() && navigable_all.is_none()
                    {
                        navigable_all = Some(started.elapsed());
                        println!(
                            "visible_thumbnail_navigation roll={roll} cache={state} visible={} camera_count={camera_count} first_camera_ms={:?} all_navigable_ms={:.6}",
                            visible.len(),
                            camera_first.map(|time| time.as_secs_f64() * 1000.),
                            started.elapsed().as_secs_f64() * 1000.
                        );
                    }
                    if ready > 0 && first.is_none() {
                        first = Some(started.elapsed());
                        first_refining = editor.refining.is_some();
                        first_settled = editor.settled.is_some();
                    }
                    if !visible.is_empty() && ready == visible.len() {
                        println!(
                            "visible_thumbnail_case roll={roll} cache={state} visible={} prepared_ms={:.6} first_ms={:.6} all_ms={:.6} first_refining={first_refining} first_settled={first_settled}",
                            visible.len(),
                            prepared.unwrap().as_secs_f64() * 1000.,
                            first.unwrap().as_secs_f64() * 1000.,
                            started.elapsed().as_secs_f64() * 1000.
                        );
                        break;
                    }
                    assert!(
                        std::time::Instant::now() < deadline,
                        "thumbnail import timed out: {}",
                        editor.status
                    );
                    std::thread::sleep(Duration::from_millis(2));
                }
                // Join logical thumbnail completion before the next case; native phases are not interruptible.
                editor.thumbnails.cancel();
                while editor.thumbnails.pending.is_some() || editor.thumbnails.embedded.pending() {
                    editor
                        .thumbnails
                        .embedded
                        .receive(&editor.document, &context);
                    editor.thumbnails.receive(&editor.document);
                    assert!(std::time::Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        }
    });
}
