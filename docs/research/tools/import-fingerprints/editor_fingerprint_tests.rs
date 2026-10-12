use super::*;
use crate::project::{
    Save,
    fingerprint_measurements::{self, Read},
};

fn report(
    roll: usize,
    trial: usize,
    phase: &str,
    elapsed: Duration,
    reads: &[Read],
    expected: usize,
) {
    assert_eq!(reads.len(), expected, "{phase} fingerprint count");
    for read in reads {
        assert_eq!(read.bytes, std::fs::metadata(&read.path).unwrap().len());
    }
    let hash_seconds: f64 = reads.iter().map(|read| read.elapsed.as_secs_f64()).sum();
    println!(
        "fingerprint_case roll={roll} trial={trial} phase={phase} wall_ms={:.6} hash_ms={:.6} hash_reads={} hash_bytes={}",
        elapsed.as_secs_f64() * 1000.,
        hash_seconds * 1000.,
        reads.len(),
        reads.iter().map(|read| read.bytes).sum::<u64>()
    );
    if phase == "import" || phase == "open" {
        println!(
            "fingerprint_document roll={roll} trial={trial} phase={phase} registration_hash_ms={:.6} selected_pre_hash_ms={:.6} selected_post_hash_ms={:.6}",
            reads[..roll]
                .iter()
                .map(|read| read.elapsed.as_secs_f64())
                .sum::<f64>()
                * 1000.,
            reads[roll].elapsed.as_secs_f64() * 1000.,
            reads[roll + 1].elapsed.as_secs_f64() * 1000.
        );
        assert_eq!(reads[0].path, reads[roll].path);
        assert_eq!(reads[0].path, reads[roll + 1].path);
    }
}

fn measured<T>(
    roll: usize,
    trial: usize,
    phase: &str,
    expected: usize,
    work: impl FnOnce() -> T,
) -> T {
    let started = std::time::Instant::now();
    let (result, reads) = fingerprint_measurements::capture(work);
    report(roll, trial, phase, started.elapsed(), &reads, expected);
    result
}

#[test]
#[ignore = "read-only local RAW/Metal import, switching and thumbnail fingerprint measurement"]
fn import_switching_fingerprint_latency() {
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
            "fingerprint_probe trials_per_roll=3 filesystem_cache=uncontrolled renderer=prewarmed_then_reused foreground_gpu_load=none boundary=backend_ready_not_presentation"
        );
        for roll in [4, 36] {
            let paths: Vec<_> = (4378..4378 + roll)
                .map(|number| PathBuf::from(format!("samples/DSC0{number}.ARW")))
                .collect();
            for trial in 1..=3 {
                let directory = tempfile::tempdir().unwrap();
                let mut scan = None;
                let imported = measured(roll, trial, "import", roll + 2, || {
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
                let mut ready = imported.ready.unwrap();
                let saved = imported
                    .document
                    .save(&directory.path().join("project.json"), Save::New)
                    .unwrap();
                let opened = measured(roll, trial, "open", roll + 2, || {
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
                drop(ready);
                ready = opened.ready.unwrap();
                let document = opened.document;
                assert_eq!(
                    ready.record.fingerprint,
                    document
                        .project
                        .source(document.project.order[0])
                        .unwrap()
                        .fingerprint
                );
                let wb = document.project.development.as_ref().unwrap().fixed_wb;
                for (step, index) in [roll - 1, 1, roll - 1, 0].into_iter().enumerate() {
                    let id = document.project.order[index];
                    let source = document.project.source(id).unwrap();
                    let (next, developed) =
                        measured(roll, trial, &format!("switch_{step}"), 2, || {
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
                    source: SourceKey::new(&source, &ready.development),
                    target: OutputTarget::DisplayP3,
                    field: None,
                };
                let disk_path = directory.path().join("thumbnails");
                let sources = Thumbnails::default().sources;
                let thumbnail = |phase: &str, expected: usize, sources, target| {
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
                    let started = std::time::Instant::now();
                    let worker = std::thread::spawn(move || {
                        fingerprint_measurements::capture(|| {
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
                        })
                    });
                    let outcome = receive.recv_timeout(Duration::from_secs(20)).unwrap();
                    assert!(outcome.image.unwrap().is_some());
                    let (_, reads) = worker.join().unwrap();
                    report(roll, trial, phase, started.elapsed(), &reads, expected);
                };
                thumbnail(
                    "thumbnail_cold",
                    2,
                    sources.clone(),
                    OutputTarget::DisplayP3,
                );
                thumbnail("thumbnail_memory", 0, sources, OutputTarget::Srgb);
                thumbnail(
                    "thumbnail_disk",
                    1,
                    Thumbnails::default().sources,
                    OutputTarget::DisplayP3,
                );
                let mut seeded = Thumbnails::default();
                seeded.seed(
                    &source,
                    &ready.development,
                    ready.thumbnail.clone().unwrap(),
                );
                thumbnail("thumbnail_seed", 0, seeded.sources, OutputTarget::DisplayP3);
                assert!(scan.is_some());
                drop(ready);
            }
        }
    });
}
