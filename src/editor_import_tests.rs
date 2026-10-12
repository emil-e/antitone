use super::*;
use crate::project::Save;

fn measured<T>(roll: usize, trial: usize, phase: &str, work: impl FnOnce() -> T) -> T {
    let started = std::time::Instant::now();
    let result = work();
    println!(
        "import_case roll={roll} trial={trial} phase={phase} wall_ms={:.6}",
        started.elapsed().as_secs_f64() * 1000.
    );
    result
}

#[test]
#[ignore = "read-only local RAW/Metal import, switching and thumbnail readiness measurement"]
fn import_switching_latency() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let renderers = Arc::new(RendererCache::default());
        renderers.prewarm(device.clone(), queue.clone());
        println!(
            "import_probe trials_per_roll=3 filesystem_cache=uncontrolled renderer=prewarmed_then_reused foreground_gpu_load=none boundary=backend_ready_not_presentation"
        );
        for roll in [4, 36] {
            let paths: Vec<_> = (4378..4378 + roll)
                .map(|number| PathBuf::from(format!("samples/DSC0{number}.ARW")))
                .collect();
            for trial in 1..=3 {
                let directory = tempfile::tempdir().unwrap();
                let mut scan = None;
                let imported = measured(roll, trial, "import", || {
                    prepare_document(
                        &renderers,
                        &device,
                        &queue,
                        &mut scan,
                        1,
                        DocumentWork::Import(Document::default(), paths.clone(), None),
                        None,
                    )
                    .unwrap()
                });
                assert_eq!(
                    imported.selected,
                    imported.document.project.order.first().copied()
                );
                drop(imported.ready);
                let saved = imported
                    .document
                    .save(&directory.path().join("project.json"), Save::New)
                    .unwrap();
                let opened = measured(roll, trial, "open", || {
                    prepare_document(
                        &renderers,
                        &device,
                        &queue,
                        &mut scan,
                        2,
                        DocumentWork::Open(saved.path.clone().unwrap()),
                        None,
                    )
                    .unwrap()
                });
                assert_eq!(opened.document.project, saved.project);
                let mut ready = opened.ready.unwrap();
                let document = opened.document;
                let wb = document.project.development.as_ref().unwrap().fixed_wb;
                for (step, index) in [roll - 1, 1, roll - 1, 0].into_iter().enumerate() {
                    let id = document.project.order[index];
                    let source = document.project.source(id).unwrap();
                    let (next, developed) =
                        measured(roll, trial, &format!("switch_{step}"), || {
                            prepare_source(
                                &renderers,
                                &device,
                                &queue,
                                document.source_path(id).unwrap(),
                                Some(wb),
                                Some(source),
                                None,
                                document.project.camera_basis.as_ref(),
                            )
                            .unwrap()
                        });
                    scan = Some((step as u64 + 3, developed));
                    ready = next;
                }
                let source = document
                    .project
                    .source(document.project.order[0])
                    .unwrap()
                    .clone();
                let path = document.source_path(source.id).unwrap();
                let key = ImageKey {
                    source: SourceKey::new(&source, &ready.development, FileKey::new(&path)),
                    target: OutputTarget::DisplayP3,
                    field: None,
                };
                let disk_path = directory.path().join("thumbnails");
                let thumbnail = |phase: &str, sources, target| {
                    measured(roll, trial, phase, || {
                        let (send, commands) = mpsc::channel();
                        let (messages, receive) = mpsc::channel();
                        send.send(Job {
                            ticket: 1,
                            key: ImageKey {
                                target,
                                ..key.clone()
                            },
                            path: path.clone(),
                            source: source.clone(),
                            correction: None,
                        })
                        .unwrap();
                        drop(send);
                        let device = device.clone();
                        let queue = queue.clone();
                        let renderers = renderers.clone();
                        let disk = disk::DiskCache::at(disk_path.clone());
                        let worker = std::thread::spawn(move || {
                            worker(
                                device,
                                queue,
                                renderers,
                                sources,
                                commands,
                                messages,
                                Arc::new(AtomicU64::new(1)),
                                Some(disk),
                            );
                        });
                        let outcome = receive.recv_timeout(Duration::from_secs(20)).unwrap();
                        assert!(outcome.image.unwrap().is_some());
                        worker.join().unwrap();
                    });
                };
                let sources = Thumbnails::default().sources;
                thumbnail("thumbnail_cold", sources.clone(), OutputTarget::DisplayP3);
                thumbnail("thumbnail_memory", sources, OutputTarget::Srgb);
                thumbnail(
                    "thumbnail_disk",
                    Thumbnails::default().sources,
                    OutputTarget::DisplayP3,
                );
                let mut seeded = Thumbnails::default();
                let (file, proxy) = ready.thumbnail.clone().unwrap();
                seeded.seed(&source, &ready.development, file, proxy);
                thumbnail("thumbnail_seed", seeded.sources, OutputTarget::DisplayP3);
                assert!(scan.is_some());
            }
        }
    });
}

#[test]
#[ignore = "local RAW/Metal byte-edit acceptance and selected-proxy revision regression"]
fn raw_byte_edits_reopen_without_identity_rejection_and_old_seed_keeps_its_revision() {
    use std::io::Write;
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let renderers = RendererCache::default();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("edited.ARW");
        std::fs::copy("samples/DSC04378.ARW", &path).unwrap();
        let mut document = Document::default();
        document.import(std::slice::from_ref(&path)).unwrap();
        let id = document.project.order[0];
        let source = document.project.source(id).unwrap();
        let wb = document.project.development.as_ref().unwrap().fixed_wb;
        let (before, _) = prepare_source(
            &renderers,
            &device,
            &queue,
            path.clone(),
            Some(wb),
            Some(source),
            None,
            document.project.camera_basis.as_ref(),
        )
        .unwrap();
        let saved = document
            .save(&directory.path().join("project.json"), Save::New)
            .unwrap();
        // Only a private copy is edited. The old whole-file fingerprint would reject this trailer.
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"intentional RAW annotation")
            .unwrap();
        let reopened = Document::open(saved.path.as_ref().unwrap()).unwrap();
        assert_eq!(reopened.project, saved.project);
        assert!(!reopened.dirty());
        let (file, proxy) = before.thumbnail.unwrap();
        let current = SourceKey::new(source, &before.development, FileKey::new(&path));
        assert_ne!(file, current.file);
        let mut thumbnails = Thumbnails::default();
        thumbnails.seed(source, &before.development, file, proxy.clone());
        assert!(thumbnails.sources.lock().unwrap().get(&current).is_none());
        let (after, _) = prepare_source(
            &renderers,
            &device,
            &queue,
            path,
            Some(wb),
            Some(source),
            None,
            document.project.camera_basis.as_ref(),
        )
        .unwrap();
        assert_eq!(after.thumbnail.unwrap().1.pixels, proxy.pixels);
    });
}
