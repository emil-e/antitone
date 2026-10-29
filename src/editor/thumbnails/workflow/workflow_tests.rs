use super::*;
use crate::application::test_support::with_application;
use std::sync::atomic::{AtomicUsize, Ordering};

fn document(count: usize) -> (Document, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng");
    let paths: Vec<_> = (0..count)
        .map(|index| {
            let path = directory.path().join(format!("source-{index}.dng"));
            std::fs::copy(&fixture, &path).unwrap();
            path
        })
        .collect();
    let mut document = Document::default();
    document.import(&paths).unwrap();
    (document, directory)
}

fn editor(
    document: Document,
    disk: &std::path::Path,
    scheduler: source_loading::SourceLoader,
) -> Editor {
    pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let mut editor = crate::editor::tests::fixture();
        editor.document = editor.document.replacement(document);
        editor.set_dmin(editor.document.state.project.dmin().map(DVec3::from_array));
        editor.set_dmax(editor.document.state.project.dmax().map(DVec3::from_array));
        editor.document.roll = editor.document.state.project.roll;
        editor.document.roll_orientation = editor.document.state.project.orientation;
        editor.document.thumbnails = (Thumbnails::with_developments(
            device,
            queue,
            Arc::default(),
            Some(disk::DiskCache::at(disk.to_owned())),
            scheduler.clone(),
        ))
        .with_sources(editor.document.cache.clone());
        editor
            .document
            .thumbnails
            .set_limits(cache_limits::CacheLimits {
                cpu_bytes: 0,
                gpu_bytes: 0,
            });
        editor.document.visit.canvas.grade.original.last_edit =
            std::time::Instant::now() - Duration::from_secs(1);
        editor.developments = Some(scheduler);
        editor
    })
}

fn begin(editor: &mut Editor, access: &crate::application::UiContext<'_, Editor>, demand: Demand) {
    editor.document.thumbnails.demand = demand;
    editor.advance_thumbnail_demand(access);
}

fn gated(
    document: &Document,
) -> (
    source_loading::SourceLoader,
    mpsc::Receiver<()>,
    mpsc::Sender<()>,
    Arc<AtomicUsize>,
) {
    let path = document.source_path(document.project.order[0]).unwrap();
    let native = Scan::open(
        &path,
        Some(document.project.development().as_ref().unwrap().fixed_wb),
    )
    .unwrap()
    .source()
    .clone();
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let scheduler = source_loading::SourceLoader::test_executor(move |_| {
        if observed.fetch_add(1, Ordering::Relaxed) == 0 {
            entered.send(()).unwrap();
            let _ = resume.recv();
        }
        Ok(Some(native.clone()))
    });
    (scheduler, starts, release, calls)
}

#[test]
fn visible_then_eager_demand_finishes_without_paint_at_zero_optional_limits() {
    let (document, _sources) = document(3);
    let first = document.project.order[0];
    let disk = tempfile::tempdir().unwrap();
    let scheduler = source_loading::SourceLoader::new();
    let editor = editor(document, disk.path(), scheduler.clone());
    with_application(editor, |app| {
        app.enter(|editor, access| begin(editor, access, Demand::Visible(vec![first])));
        assert!(app.read(|editor| editor.document.thumbnail_work.pending()));
        app.run_until(Duration::from_secs(30), |editor| {
            !editor.document.thumbnail_work.pending()
        });
        app.read(|editor| {
            assert_eq!(editor.document.thumbnails.attempted_sources.len(), 3);
            assert_eq!(
                editor
                    .document
                    .thumbnails
                    .ready_count(&editor.document, editor.document.target),
                1
            );
            assert!(editor.document.thumbnails.pending.is_none());
            assert_eq!(editor.document.cache.charges().retained, 0);
            for id in &editor.document.state.project.order {
                let key = tests::key(&editor.document, *id);
                assert!(
                    disk::DiskCache::at(disk.path().to_owned())
                        .load(&key.source)
                        .unwrap()
                        .is_some()
                );
            }
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn composed_editor_registers_visible_work_then_uses_completed_proper_card_callbacks() {
    let (document, _sources) = document(2);
    let disk = tempfile::tempdir().unwrap();
    let scheduler = source_loading::SourceLoader::new();
    let editor = editor(document, disk.path(), scheduler.clone());
    let context = egui::Context::default();
    with_application(editor, |app| {
        for _ in 0..4 {
            app.enter(|editor, access| {
                let mut output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1100., 800.),
                        )),
                        ..Default::default()
                    },
                    |ui| crate::platform::host::App::ui(editor, ui, access),
                );
                output.textures_delta.clear();
            });
            if app.read(|editor| editor.document.thumbnail_work.pending()) {
                break;
            }
        }
        assert!(app.read(|editor| editor.document.thumbnail_work.pending()));
        app.run_until(Duration::from_secs(30), |editor| {
            !editor.document.thumbnail_work.pending() && editor.document.embedded_preview.is_none()
        });
        app.read(|editor| {
            assert_eq!(
                editor
                    .document
                    .thumbnails
                    .ready_count(&editor.document, editor.document.target),
                2
            );
            assert!(
                editor.document.visit.canvas.ready.is_none()
                    && editor.document.visit.pending_preview.is_none()
            );
        });
        let mut output = app.enter(|editor, access| {
            context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100., 800.),
                    )),
                    ..Default::default()
                },
                |ui| crate::platform::host::App::ui(editor, ui, access),
            )
        });
        output.textures_delta.clear();
        assert!(
            output
                .shapes
                .iter()
                .filter(|shape| matches!(&shape.shape, egui::Shape::Callback(_)))
                .count()
                >= 2,
            "the two completed cards must supply image callbacks"
        );
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn a_closed_producer_retires_its_request_and_reports_failure_for_current_and_later_visible_cards() {
    let (document, _sources) = document(3);
    let ids = document.project.order.clone();
    let disk = tempfile::tempdir().unwrap();
    let scheduler = source_loading::SourceLoader::new();
    let mut editor = editor(document, disk.path(), scheduler.clone());
    let (commands, requests) = mpsc::channel();
    editor.document.thumbnails.services = Some(Services::Controlled(commands));
    with_application(editor, |app| {
        app.enter(|editor, access| begin(editor, access, Demand::Visible(ids[..2].to_vec())));
        app.pump();
        let submission = requests.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(submission);
        app.run_until(Duration::from_secs(5), |editor| {
            !editor.document.thumbnail_work.pending()
        });
        app.read(|editor| {
            assert!(editor.document.thumbnails.services.is_none());
            assert!(editor.document.thumbnails.pending.is_none());
            for id in &ids[..2] {
                let key = tests::key(&editor.document, *id);
                assert!(
                    editor
                        .document
                        .thumbnails
                        .cache
                        .entries
                        .iter()
                        .any(|(cached, image)| cached == &key
                            && image
                                .as_ref()
                                .is_err_and(|error| error.contains("worker stopped")))
                );
            }
        });
        app.enter(|editor, cx| {
            editor.document.thumbnails.demand = Demand::Visible(vec![ids[2]]);
            editor.advance_thumbnail_demand(cx);
            let key = tests::key(&editor.document, ids[2]);
            assert!(
                editor
                    .document
                    .thumbnails
                    .cache
                    .entries
                    .iter()
                    .any(|(cached, image)| cached == &key
                        && image
                            .as_ref()
                            .is_err_and(|error| error.contains("worker stopped")))
            );
            assert!(editor.document.thumbnails.pending.is_none());
            assert!(!editor.document.thumbnail_work.pending());
        });
        assert!(
            requests.try_recv().is_err(),
            "unavailable services must not receive another request"
        );
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn hidden_document_demand_completes_cpu_disk_work_without_gpu_residency_or_paint() {
    let (document, _sources) = document(3);
    let disk = tempfile::tempdir().unwrap();
    let scheduler = source_loading::SourceLoader::new();
    let editor = editor(document, disk.path(), scheduler.clone());
    with_application(editor, |app| {
        app.enter(|editor, access| begin(editor, access, Demand::Hidden));
        app.run_until(Duration::from_secs(30), |editor| {
            !editor.document.thumbnail_work.pending()
        });
        app.read(|editor| {
            assert_eq!(editor.document.thumbnails.attempted_sources.len(), 3);
            assert!(editor.document.thumbnails.cache.entries.is_empty());
            assert_eq!(editor.document.cache.charges().retained, 0);
            for id in &editor.document.state.project.order {
                let key = tests::key(&editor.document, *id);
                assert!(
                    disk::DiskCache::at(disk.path().to_owned())
                        .load(&key.source)
                        .unwrap()
                        .is_some()
                );
            }
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn retiring_visible_gpu_demand_keeps_document_cpu_completion_and_disk_publication() {
    let (document, _sources) = document(1);
    let first = document.project.order[0];
    let (scheduler, starts, release, calls) = gated(&document);
    let disk = tempfile::tempdir().unwrap();
    let editor = editor(document, disk.path(), scheduler.clone());
    with_application(editor, |app| {
        app.enter(|editor, access| begin(editor, access, Demand::Visible(vec![first])));
        app.pump();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while starts.try_recv().is_err() {
            app.pump();
            assert!(
                std::time::Instant::now() < deadline,
                "native acquisition did not start"
            );
            std::thread::yield_now();
        }
        app.update(|editor| {
            editor.document.thumbnails.cancel();
            editor.document.thumbnails.demand = Demand::Hidden;
            editor.document.thumbnails.refresh_hidden_memory();
        });
        assert!(app.read(|editor| editor.document.thumbnail_work.pending()));
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(30), |editor| {
            !editor.document.thumbnail_work.pending()
        });
        app.read(|editor| {
            let key = tests::key(&editor.document, first);
            assert!(
                editor
                    .document
                    .thumbnails
                    .attempted_sources
                    .contains(&key.source)
            );
            assert!(editor.document.thumbnails.cache.entries.is_empty());
            assert!(
                disk::DiskCache::at(disk.path().to_owned())
                    .load(&key.source)
                    .unwrap()
                    .is_some()
            );
            assert_eq!(editor.document.cache.charges().retained, 0);
        });
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn document_replacement_retires_old_publication_without_stopping_the_shared_worker() {
    let (first, _first_sources) = document(1);
    let (second, _second_sources) = document(1);

    let old_key = tests::key(&first, first.project.order[0]);
    let (scheduler, starts, release, calls) = gated(&first);
    let disk = tempfile::tempdir().unwrap();
    let editor = editor(first, disk.path(), scheduler.clone());
    let old_store = Arc::downgrade(&editor.document.cache);
    with_application(editor, |app| {
        app.enter(|editor, access| begin(editor, access, Demand::Hidden));
        app.pump();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while starts.try_recv().is_err() {
            app.pump();
            assert!(
                std::time::Instant::now() < deadline,
                "native acquisition did not start"
            );
            std::thread::yield_now();
        }
        app.enter(|editor, access| {
            editor.document = editor.document.replacement(second);
            begin(editor, access, Demand::Hidden);
        });
        app.pump();
        assert!(old_store.upgrade().is_none());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(30), |editor| {
            !editor.document.thumbnail_work.pending()
        });
        app.read(|editor| {
            let current = tests::key(&editor.document, editor.document.state.project.order[0]);
            assert_eq!(
                editor.document.thumbnails.attempted_sources,
                vec![current.source.clone()]
            );
            assert!(
                disk::DiskCache::at(disk.path().to_owned())
                    .load(&current.source)
                    .unwrap()
                    .is_some()
            );
            assert!(editor.document.thumbnails.cache.entries.is_empty());
        });
        assert!(
            disk::DiskCache::at(disk.path().to_owned())
                .load(&old_key.source)
                .unwrap()
                .is_none()
        );
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn document_retirement_drops_thumbnail_demand_queued_behind_unrelated_native_work() {
    let (document, _sources) = document(1);
    let id = document.project.order[0];
    let path = document.source_path(id).unwrap();
    let (scheduler, starts, release, calls) = gated(&document);
    let unrelated = scheduler
        .submit(
            source_loading::Request {
                path,
                wb: None,
                expected: None,
                key: None,
            },
            source_loading::Priority::Foreground,
        )
        .unwrap();
    starts.recv_timeout(Duration::from_secs(5)).unwrap();
    let disk = tempfile::tempdir().unwrap();
    let editor = editor(document, disk.path(), scheduler.clone());
    with_application(editor, |app| {
        app.enter(|editor, access| begin(editor, access, Demand::Visible(vec![id])));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while scheduler.waiter_count() != 2 {
            app.pump();
            assert!(
                std::time::Instant::now() < deadline,
                "thumbnail did not queue its source demand"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        app.update(|editor| {
            editor.document = editor.document.replacement(Document::default());
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while scheduler.waiter_count() != 1 {
            app.pump();
            assert!(
                std::time::Instant::now() < deadline,
                "retired thumbnail still owns native demand"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        release.send(()).unwrap();
        futures_lite::future::block_on(unrelated).unwrap();
    });
    scheduler.close_and_wait().unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[test]
fn application_retirement_drops_waiters_but_running_native_work_finishes_before_cleanup() {
    let (document, _sources) = document(1);

    let key = tests::key(&document, document.project.order[0]);
    let (scheduler, starts, release, calls) = gated(&document);
    let disk = tempfile::tempdir().unwrap();
    let editor = editor(document, disk.path(), scheduler.clone());
    let store = Arc::downgrade(&editor.document.cache);
    with_application(editor, |app| {
        app.enter(|editor, access| begin(editor, access, Demand::Hidden));
        app.pump();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while starts.try_recv().is_err() {
            app.pump();
            assert!(
                std::time::Instant::now() < deadline,
                "native acquisition did not start"
            );
            std::thread::yield_now();
        }
        app.close_execution();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        app.read(|editor| assert!(editor.document.thumbnails.attempted_sources.is_empty()));
    });
    assert!(store.upgrade().is_none());
    release.send(()).unwrap();
    scheduler.close_and_wait().unwrap();
    assert!(
        disk::DiskCache::at(disk.path().to_owned())
            .load(&key.source)
            .unwrap()
            .is_none()
    );
}
