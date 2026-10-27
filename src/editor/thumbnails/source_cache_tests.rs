use super::*;
use crate::project::Development;
use crate::raw::fixture as raw_fixture;
use crate::{scan::CameraProxy, source::SourceKey};

mod dust_worker;
mod lazy_worker;

const BUDGET: u64 = 8 << 20;

fn document(path: &std::path::Path) -> super::document_scope::DocumentScope {
    let mut document = Document::default();
    let (source, _) = document::open_source(path, Some([1.; 4])).unwrap();
    let development = Development::for_source(&source.development, [1.; 4]);
    document
        .register_with_development(source, development)
        .unwrap();
    let document: super::document_scope::DocumentScope = document.into();
    document.cache.set_budget(BUDGET);
    document
}

fn wb(document: &Document) -> Option<[f64; 4]> {
    document
        .project
        .development()
        .map(|development| development.fixed_wb)
}

fn codes(scan: &Scan) -> Vec<[f32; 4]> {
    let image = scan.source().camera_image().unwrap();
    (0..image.height)
        .flat_map(|y| (0..image.width).map(move |x| (x, y)))
        .map(|(x, y)| image.pixel_samples(x, y).unwrap())
        .collect()
}

fn key(document: &Document) -> SourceKey {
    let id = document.project.order[0];
    SourceKey::capture(
        &document.source_path(id).unwrap(),
        &document.project.source(id).unwrap().development,
    )
    .unwrap()
}

async fn gpu() -> (wgpu::Device, wgpu::Queue) {
    let instance = crate::gpu::instance();
    instance
        .request_adapter(&Default::default())
        .await
        .unwrap()
        .request_device(&Default::default())
        .await
        .unwrap()
}

fn prepare(
    document: &Document,
    cache: &Arc<crate::source::SourceCache>,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderers: &crate::renderer::RendererCache,
) -> (Ready, Scan) {
    let id = document.project.order[0];
    let path = document.source_path(id).unwrap();
    let expected = document.project.source(id).unwrap();
    let wb = wb(document);
    let file = document.file_key(&path);
    let prepared = prepare_source_cached(
        renderers,
        device,
        queue,
        path,
        wb,
        Some(expected),
        None,
        Some((cache, file)),
    )
    .unwrap();
    assert_eq!(
        prepared.0.analysis_support, None,
        "ordinary source preparation must not request support"
    );
    prepared
}

#[test]
fn discarding_a_candidate_snapshot_keeps_the_scope_cache_and_live_pixels() {
    let file = raw_fixture::bayer_dng();
    let document = document(file.path());
    let key = key(&document);
    let scan = Scan::open(file.path(), wb(&document)).unwrap();
    document
        .cache
        .publish_source(key.clone(), scan.source().clone())
        .unwrap();
    let before = document.cache.charges();
    let candidate = document.capture();
    drop(candidate);

    assert!(!document.cache.is_closed());
    assert_eq!(document.cache.charges(), before);
    let consumer = Scan::from_source(document.cache.get(&key).unwrap());
    assert!(Arc::ptr_eq(consumer.source(), scan.source()));
    assert_eq!(codes(&consumer), codes(&scan));
}

#[test]
fn same_document_install_keeps_protected_proxies_at_zero_optional_budget() {
    let file = raw_fixture::bayer_dng();
    let mut document = document(file.path());
    document.cache.set_budget(0);
    let cache = document.cache.clone();
    let key = key(&document);
    let scan = Scan::open(file.path(), wb(&document)).unwrap();
    let proxy = Arc::new(CameraProxy::from_scan(&scan).unwrap());
    cache
        .publish_source(key.clone(), scan.source().clone())
        .unwrap();
    cache.protect_proxies(vec![key.clone()]);
    cache.publish_proxy(key.clone(), proxy.clone()).unwrap();
    document.install(document.capture());
    assert!(Arc::ptr_eq(&document.cache, &cache));
    assert!(!cache.is_closed());
    assert_eq!(cache.charges().retained, 0);
    assert_eq!(cache.protected_bytes(), proxy.retained_bytes());
    assert!(Arc::ptr_eq(&cache.get_proxy(&key).unwrap(), &proxy));
    assert!(Arc::ptr_eq(&cache.get(&key).unwrap(), scan.source()));
}

#[test]
fn foreground_and_candidate_reads_share_the_scope_cache() {
    pollster::block_on(async {
        let file = raw_fixture::bayer_dng();
        let document = document(file.path());
        let (device, queue) = gpu().await;
        let renderers = crate::renderer::RendererCache::default();
        let (first, scan) = prepare(&document, &document.cache, &device, &queue, &renderers);
        let (_, warm) = prepare(&document, &document.cache, &device, &queue, &renderers);
        assert!(Arc::ptr_eq(scan.source(), warm.source()));
        assert_eq!(codes(&scan), codes(&warm));
        assert_eq!(
            first.development().clone(),
            document
                .project
                .source(document.project.order[0])
                .unwrap()
                .development
                .decode
        );
        let before = document.cache.charges();
        let candidate = document.capture();
        let (_, borrowed) = prepare(&candidate, &document.cache, &device, &queue, &renderers);
        assert!(Arc::ptr_eq(scan.source(), borrowed.source()));

        assert_eq!(document.cache.charges(), before);
        drop(candidate);
        assert!(document.cache.get(&key(&document)).is_some());
    });
}

#[test]
fn cpu_only_proper_work_persists_disk_at_zero_limits_then_materializes_on_demand() {
    pollster::block_on(async {
        let file = raw_fixture::bayer_dng();
        let document = document(file.path());
        let directory = tempfile::tempdir().unwrap();
        let disk = disk::DiskCache::at(directory.path().to_owned());
        let (device, queue) = gpu().await;
        let mut thumbnails = Thumbnails::with_disk(
            device,
            queue,
            Arc::default(),
            Some(disk::DiskCache::at(directory.path().to_owned())),
        );
        thumbnails.apply_budgets(memory::CacheBudgets::new(
            cache_limits::CacheLimits {
                cpu_bytes: 0,
                gpu_bytes: 0,
            },
            memory::Pressure::Normal,
        ));
        thumbnails.bind_sources(document.cache.clone());
        let id = document.project.order[0];
        let source = document.project.source(id).unwrap();
        let path = document.source_path(id).unwrap();
        let image_key = thumbnails::ImageKey {
            source: thumbnails::SourceKey::new(source, thumbnails::FileKey::new(&path)),
            target: OutputTarget::DisplayP3,
            field: None,
            recipe: Ok(None),
            projection: None,
        };
        let scan = Scan::open(&path, wb(&document)).unwrap();
        let seed = Arc::new(CameraProxy::from_scan(&scan).unwrap());
        thumbnails.seed_at_path(source, &path, thumbnails::FileKey::new(&path), seed.clone());
        assert!(
            !thumbnails.attempted_sources.contains(&image_key.source),
            "a seed is not yet a durable completed attempt"
        );
        thumbnails.release_selected();
        let job = tests::service_job(
            &thumbnails,
            image_key.clone(),
            path.clone(),
            source.clone(),
            None,
            false,
        );
        tests::complete_service_request(&mut thumbnails, &document, job);
        assert!(thumbnails.pending.is_none());
        assert!(thumbnails.attempted_sources.contains(&image_key.source));
        assert!(thumbnails.cache.entries.is_empty());
        assert_eq!(document.cache.charges().retained, 0);
        assert_eq!(document.cache.protected_bytes(), 0);
        assert!(disk.load(&image_key.source).unwrap().is_some());
        drop(seed);
        thumbnails.cache.protect(vec![image_key.clone()]);
        document.cache.protect_proxies(vec![key(&document)]);
        let job = tests::service_job(
            &thumbnails,
            image_key.clone(),
            path,
            source.clone(),
            None,
            true,
        );
        tests::complete_service_request(&mut thumbnails, &document, job);
        assert!(thumbnails.pending.is_none());
        assert!(thumbnails.cache.get(&image_key).unwrap().is_ok());
        assert_eq!(document.cache.charges().retained, 0);
        assert!(document.cache.protected_bytes() > 0);
    });
}

#[test]
fn background_native_by_product_is_a_foreground_hit_under_the_shared_cpu_limit() {
    pollster::block_on(async {
        let file = raw_fixture::bayer_dng();
        let document = document(file.path());
        let (device, queue) = gpu().await;
        let renderers = Arc::new(crate::renderer::RendererCache::default());
        let mut thumbnails =
            Thumbnails::with_disk(device.clone(), queue.clone(), renderers.clone(), None);
        thumbnails.apply_budgets(memory::CacheBudgets::new(
            cache_limits::CacheLimits {
                cpu_bytes: BUDGET,
                gpu_bytes: 0,
            },
            memory::Pressure::Normal,
        ));
        thumbnails.bind_sources(document.cache.clone());
        let id = document.project.order[0];
        let source = document.project.source(id).unwrap();
        let path = document.source_path(id).unwrap();
        let image_key = thumbnails::ImageKey {
            source: thumbnails::SourceKey::new(source, thumbnails::FileKey::new(&path)),
            target: OutputTarget::DisplayP3,
            field: None,
            recipe: Ok(None),
            projection: None,
        };
        // Exercise the same serial worker and publication path used by filmstrip demand.
        let job = tests::service_job(&thumbnails, image_key, path, source.clone(), None, true);
        let outcome = tests::run_service_request(&thumbnails, job).unwrap();
        assert!(outcome.image.unwrap().is_some());
        let native = document.cache.get(&key(&document)).unwrap();
        assert!(document.cache.get_proxy(&key(&document)).is_some());
        assert!(document.cache.charges().retained <= BUDGET);
        let (_, foreground) = prepare(&document, &document.cache, &device, &queue, &renderers);
        assert!(Arc::ptr_eq(&native, foreground.source()));
        document.cache.set_budget(0);
        assert_eq!(document.cache.charges().retained, 0);
        assert!(Arc::ptr_eq(
            &document.cache.get(&key(&document)).unwrap(),
            foreground.source()
        ));
    });
}

#[test]
fn cancelled_gpu_ticket_keeps_proper_disk_demand_and_shares_foreground_handoff() {
    pollster::block_on(async {
        let file = raw_fixture::bayer_dng();
        let document = document(file.path());
        document.cache.set_budget(0);
        let native = Scan::open(file.path(), wb(&document))
            .unwrap()
            .source()
            .clone();
        let worker_native = native.clone();
        let (entered, starts) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let worker_calls = calls.clone();
        let developments = source_loading::SourceLoader::test_executor(move |_| {
            worker_calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            entered.send(()).unwrap();
            resume.recv().unwrap();
            Ok(Some(worker_native.clone()))
        });
        let (device, queue) = gpu().await;
        let renderers = Arc::new(crate::renderer::RendererCache::default());
        let directory = tempfile::tempdir().unwrap();
        let disk = disk::DiskCache::at(directory.path().to_owned());
        let mut thumbnails = Thumbnails::with_developments(
            device.clone(),
            queue.clone(),
            renderers.clone(),
            Some(disk::DiskCache::at(directory.path().to_owned())),
            developments.clone(),
        );
        thumbnails.apply_budgets(memory::CacheBudgets::new(
            cache_limits::CacheLimits {
                cpu_bytes: 0,
                gpu_bytes: 0,
            },
            memory::Pressure::Normal,
        ));
        thumbnails.bind_sources(document.cache.clone());
        let id = document.project.order[0];
        let path = document.source_path(id).unwrap();
        let expected = document.project.source(id).unwrap();
        let key = ImageKey {
            source: thumbnails::SourceKey::new(expected, FileKey::new(&path)),
            target: OutputTarget::Srgb,
            field: None,
            recipe: Ok(None),
            projection: None,
        };
        let job = tests::service_job(
            &thumbnails,
            key.clone(),
            path.clone(),
            expected.clone(),
            None,
            true,
        );
        crate::application::test_support::with_application(Vec::new(), |app| {
            let _task = tests::start_service_request(&thumbnails, job, &app.access());
            app.pump();
            starts.recv_timeout(Duration::from_secs(5)).unwrap();
            thumbnails.cancel();
            let cache = &document.cache;
            let file = document.file_key(&path);
            std::thread::scope(|scope| {
                let foreground = scope.spawn(|| {
                    prepare_source_scheduled(
                        &renderers,
                        &device,
                        &queue,
                        path.clone(),
                        wb(&document),
                        Some(expected),
                        None,
                        Some((cache, file.clone())),
                        &developments,
                        &|| false,
                    )
                });
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                while developments.waiter_count() != 2 {
                    assert!(std::time::Instant::now() < deadline);
                    std::thread::yield_now();
                }
                release.send(()).unwrap();
                let (_, scan) = foreground.join().unwrap().unwrap();
                assert!(Arc::ptr_eq(scan.source(), &native));
                app.run_until(Duration::from_secs(5), |results| !results.is_empty());
                let completed = app.update(|results| results.pop().unwrap()).unwrap();
                assert!(completed.proper);
                assert!(completed.image.unwrap().is_none());
                assert!(disk.load(&key.source).unwrap().is_some());
                assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
                assert_eq!(document.cache.charges().retained, 0);
            });
        });
    });
}

#[test]
fn newly_visible_native_demand_preempts_an_eager_waiter_at_the_phase_boundary() {
    pollster::block_on(async {
        let a = raw_fixture::bayer_dng();
        let b = raw_fixture::bayer_dng();
        let mut document = document(a.path());
        let (record, _) = document::open_source(b.path(), Some([1.; 4])).unwrap();
        let development = Development::for_source(&record.development, [1.; 4]);
        document
            .register_with_development(record, development)
            .unwrap();
        document.cache.set_budget(0);
        let native_a = Scan::open(a.path(), wb(&document))
            .unwrap()
            .source()
            .clone();
        let native_b = Scan::open(b.path(), wb(&document))
            .unwrap()
            .source()
            .clone();
        let path_a = document.source_path(document.project.order[0]).unwrap();
        let path_b = document.source_path(document.project.order[1]).unwrap();
        let worker_a = path_a.clone();
        let worker_b = path_b.clone();
        let (entered, starts) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let trace = Arc::new(std::sync::Mutex::new(Vec::new()));
        let worker_trace = trace.clone();
        let mut phases = std::collections::HashMap::new();
        let developments = source_loading::SourceLoader::test_executor(move |path| {
            let phase = phases.entry(path.to_owned()).or_insert(0);
            *phase += 1;
            worker_trace.lock().unwrap().push((path.to_owned(), *phase));
            if path == worker_a && *phase == 1 {
                entered.send(()).unwrap();
                resume.recv().unwrap();
            }
            Ok((*phase == 2).then(|| {
                if path == worker_a {
                    native_a.clone()
                } else {
                    assert_eq!(path, worker_b);
                    native_b.clone()
                }
            }))
        });
        let (device, queue) = gpu().await;
        let directory = tempfile::tempdir().unwrap();
        let mut thumbnails = Thumbnails::with_developments(
            device,
            queue,
            Arc::default(),
            Some(disk::DiskCache::at(directory.path().to_owned())),
            developments.clone(),
        );
        thumbnails.apply_budgets(memory::CacheBudgets::new(
            cache_limits::CacheLimits {
                cpu_bytes: 0,
                gpu_bytes: 0,
            },
            memory::Pressure::Normal,
        ));
        thumbnails.bind_sources(document.cache.clone());
        let image_key = |index| {
            let source = document
                .project
                .source(document.project.order[index])
                .unwrap();
            let path = document.source_path(source.id).unwrap();
            ImageKey {
                source: thumbnails::SourceKey::new(source, FileKey::new(&path)),
                target: OutputTarget::Srgb,
                field: None,
                recipe: Ok(None),
                projection: None,
            }
        };
        let job = tests::service_job(
            &thumbnails,
            image_key(0),
            path_a.clone(),
            document
                .project
                .source(document.project.order[0])
                .unwrap()
                .clone(),
            None,
            false,
        );
        crate::application::test_support::with_application(Vec::new(), |app| {
            let _eager = tests::start_service_request(&thumbnails, job, &app.access());
            app.pump();
            starts.recv_timeout(Duration::from_secs(5)).unwrap();
            thumbnails.cancel();
            let job = tests::service_job(
                &thumbnails,
                image_key(1),
                path_b.clone(),
                document
                    .project
                    .source(document.project.order[1])
                    .unwrap()
                    .clone(),
                None,
                true,
            );
            let _visible = tests::start_service_request(&thumbnails, job, &app.access());
            app.pump();
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while developments.waiter_count() != 2 {
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }
            release.send(()).unwrap();
            app.run_until(Duration::from_secs(5), |results| results.len() == 2);
            let mut outcomes = app.update(std::mem::take);
            let index = outcomes
                .iter()
                .position(|outcome| outcome.as_ref().unwrap().key == image_key(1))
                .unwrap();
            let visible = outcomes.swap_remove(index).unwrap();
            let eager = outcomes.pop().unwrap().unwrap();
            assert_eq!(visible.key, image_key(1));
            assert!(visible.proper && visible.image.unwrap().is_some());
            assert_eq!(eager.key, image_key(0));
            assert!(eager.proper && eager.image.unwrap().is_none());
        });
        assert_eq!(
            *trace.lock().unwrap(),
            [
                (path_a.clone(), 1),
                (path_b.clone(), 1),
                (path_b, 2),
                (path_a, 2)
            ]
        );
    });
}

#[test]
fn acquisition_requests_use_stage_wb_and_preserve_saved_source_wb() {
    let file = raw_fixture::bayer_dng();
    let mut editor = Editor::state();
    let scan_wb = crate::project::UNITY_WB;
    editor
        .document
        .register(document::read_source(file.path(), Some(scan_wb)).unwrap())
        .unwrap();
    editor.document.validate().unwrap();
    let fresh = file.path().with_file_name("new-reference.dng");
    let saved_wb = [1.5, 1., 2., 1.];
    for digital in [false, true] {
        if digital {
            editor.document.project.input = crate::processing::InputSettings::Digital;
            editor.document.project.sources[0].development =
                document::read_source(file.path(), Some(saved_wb))
                    .unwrap()
                    .development;
            editor.document.validate().unwrap();
        }
        for request in [
            editor.reference_preview_input(fresh.clone()),
            editor.source_input(fresh.clone()),
        ] {
            assert_eq!(
                request.wb,
                if digital {
                    None
                } else {
                    Some(crate::project::UNITY_WB)
                }
            );
            assert!(request.expected.is_none());
        }
        for request in [
            editor.reference_preview_input(file.path().to_owned()),
            editor.source_input(file.path().to_owned()),
        ] {
            assert_eq!(request.wb, Some(if digital { saved_wb } else { scan_wb }));
            assert!(request.expected.is_some());
        }
    }
}

#[test]
fn registered_flat_reference_carries_its_document_native_reuse_key() {
    let file = raw_fixture::bayer_dng();
    let document = document(file.path());
    let mut editor = Editor::state();
    let expected_key = key(&document);
    editor.document = document;
    let request = editor.reference_preview_input(file.path().to_owned());
    assert_eq!(request.load_key().unwrap(), expected_key);
    let cache = request
        .cache
        .expect("registered reference uses the scope cache");
    assert!(Arc::ptr_eq(&cache, &editor.document.cache));
}
