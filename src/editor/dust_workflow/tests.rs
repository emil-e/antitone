use super::*;
use crate::{
    application::test_support::{TestApplication, with_application},
    dust_repair::{DonorChoice, RepairId, RepairOrigin, SupportMask},
    editor::{document::Document, source_loading},
    geometry::SourceRegion,
};
use std::{
    path::PathBuf,
    sync::{Mutex, mpsc},
    time::Duration,
};

const LIMIT: Duration = Duration::from_secs(30);

fn fixture(
    gate: crate::platform::SubmissionGate,
    screening: Arc<crate::gpu::dust_screening::Cache>,
) -> (Editor, source_loading::SourceLoader) {
    let (device, queue) = pollster::block_on(async {
        let instance = crate::gpu::instance();
        instance
            .request_adapter(&Default::default())
            .await
            .unwrap()
            .request_device(&Default::default())
            .await
            .unwrap()
    });
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/raw/illuminated-patches.dng");
    let mut document = Document::default();
    document.import(std::slice::from_ref(&path)).unwrap();
    let source_id = document.project.order[0];
    let scheduler = source_loading::SourceLoader::new();
    let mut e = crate::editor::tests::fixture();
    let services = source::Services {
        developments: scheduler.clone(),
        device: device.clone(),
        queue: queue.clone(),
        renderers: Arc::new(crate::renderer::RendererCache::new(gate.clone())),
    };
    let ready = futures_lite::future::block_on(services.clone().prepare(
        source::Request {
            input: document.project.input,
            path: path.clone(),
            wb: Some(document.project.development().as_ref().unwrap().fixed_wb),
            expected: Some(document.project.source(source_id).unwrap().clone()),
            cache: Some(e.document.cache.clone()),
            file: document.file_key(&path),
            correction: None,
        },
        Arc::new(AtomicBool::new(false)),
    ))
    .unwrap();
    e.document.install(document);
    e.id = 1;
    e.source_epoch.store(1, Ordering::Relaxed);
    e.replace_photo_scope(1);
    e.current_source = Some(source_id);
    e.document.visit.ready = Some(ready);
    e.source_services = Some(services);
    e.dust_services = Some(Services::new(device, queue, gate, screening));
    e.set_dmin(Some(glam::DVec3::splat(0.8)));
    e.set_dmax(Some(glam::DVec3::splat(0.02)));
    e.set_floor(crate::processing::CAMERA_INPUT_FLOOR);
    e.source = false;
    e.sync_document();
    (e, scheduler)
}

fn repair(e: &mut Editor, region: SourceRegion, donor: DonorChoice) -> RepairId {
    let source = e.current_source.unwrap();
    e.dust_repairs_mut(source)
        .insert(
            SupportMask::new(
                [region.x, region.y],
                [region.width, region.height],
                vec![true; (region.width * region.height) as usize],
            )
            .unwrap(),
            RepairOrigin::Manual,
            donor,
        )
        .unwrap()
}

fn manual(e: &mut Editor, x: u32) -> RepairId {
    repair(
        e,
        SourceRegion {
            x,
            y: 60,
            width: 3,
            height: 3,
        },
        DonorChoice::Offset([30, 0]),
    )
}

fn automatic(e: &mut Editor) -> RepairId {
    repair(
        e,
        SourceRegion {
            x: 104,
            y: 44,
            width: 32,
            height: 32,
        },
        DonorChoice::Automatic,
    )
}

fn start(app: &TestApplication<'_, Editor>) {
    app.enter(|e, access| {
        e.schedule_dust(e.dust_signature().unwrap(), access);
    });
}

fn gated() -> (
    crate::platform::SubmissionGate,
    Arc<AtomicBool>,
    mpsc::Receiver<()>,
    mpsc::Sender<()>,
) {
    let gate = crate::platform::SubmissionGate::default();
    let armed = Arc::new(AtomicBool::new(false));
    let flag = armed.clone();
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Mutex::new(resume);
    gate.observe_submissions(Arc::new(move || {
        if flag.swap(false, Ordering::AcqRel) {
            entered.send(()).unwrap();
            resume.lock().unwrap().recv_timeout(LIMIT).unwrap();
        }
    }));
    (gate, armed, starts, release)
}

#[test]
fn enabled_repairs_complete_sequentially_without_paint_or_changing_saved_instructions() {
    let (mut e, scheduler) = fixture(Default::default(), Arc::default());
    let first = manual(&mut e, 80);
    let second = manual(&mut e, 100);
    let source = e.current_source.unwrap();
    let before = e.document.project.clone();
    with_application(e, |app| {
        start(app);
        app.run_until(LIMIT, |e| e.document.dust.has_evaluation(source, second));
        app.read(|e| {
            assert!(e.document.dust.has_evaluation(source, first));
            assert!(e.document.dust.pending_request().is_none());
            assert_eq!(e.document.project, before);
            assert_eq!(e.dust_render_state(), "Repairs");
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn actual_failure_is_memoized_without_blocking_later_enabled_repairs() {
    let (mut e, scheduler) = fixture(Default::default(), Arc::default());
    let failed = repair(
        &mut e,
        SourceRegion {
            x: 80,
            y: 60,
            width: 3,
            height: 3,
        },
        DonorChoice::Offset([1000, 0]),
    );
    let good = manual(&mut e, 100);
    let source = e.current_source.unwrap();
    let before = e.document.project.clone();
    with_application(e, |app| {
        start(app);
        app.run_until(LIMIT, |e| e.document.dust.has_evaluation(source, good));
        app.enter(|e, cx| e.schedule_dust(e.dust_signature().unwrap(), cx));
        app.read(|e| {
            assert!(!e.document.dust.has_evaluation(source, failed));
            assert!(e.document.dust.pending_request().is_none());
            assert_eq!(e.document.project, before);
            assert_eq!(e.dust_render_state(), "Repair unavailable");
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn downstream_grading_and_viewport_navigation_keep_running_healing_and_its_captured_input() {
    let (gate, armed, starts, release) = gated();
    let (mut e, scheduler) = fixture(gate, Arc::default());
    let id = automatic(&mut e);
    let source = e.current_source.unwrap();
    with_application(e, |app| {
        armed.store(true, Ordering::Release);
        start(app);
        let request = app.read(|e| e.document.dust.pending_request().unwrap());
        app.pump();
        starts.recv_timeout(LIMIT).unwrap();
        app.enter(|e, cx| {
            e.adjustment_layer_settings.exposure = 1.;
            e.roll.exposure = 2.;
            e.viewport.zoom = Some(2.);
            e.schedule_dust(e.dust_signature().unwrap(), cx);
            e.sync_document();
        });
        let edited = app.read(|e| e.document.project.clone());
        app.read(|e| assert_eq!(e.document.dust.pending_request(), Some(request)));
        release.send(()).unwrap();
        app.run_until(LIMIT, |e| e.document.dust.has_evaluation(source, id));
        app.read(|e| assert_eq!(e.document.project, edited));
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn deleted_running_repair_cannot_publish_and_does_not_block_unrelated_cpu_work() {
    let (gate, armed, starts, release) = gated();
    let (mut e, scheduler) = fixture(gate, Arc::default());
    let old = automatic(&mut e);
    let source = e.current_source.unwrap();
    with_application(e, |app| {
        armed.store(true, Ordering::Release);
        start(app);
        app.pump();
        starts.recv_timeout(LIMIT).unwrap();
        let replacement = app.enter(|e, access| {
            e.reset_dust_repairs(source);
            let id = manual(e, 100);
            e.schedule_dust(e.dust_signature().unwrap(), access);

            id
        });
        app.pump();
        app.read(|e| {
            assert!(!e.document.dust.has_evaluation(source, replacement));
            assert!(e.document.dust.pending_request().is_some());
        });
        let (entered, observed) = mpsc::channel();
        let probe = app
            .access()
            .spawn(move |_| async move {
                crate::background::run(move || entered.send(()).unwrap())
                    .await
                    .unwrap();
            })
            .unwrap();
        app.run_until(LIMIT, |_| observed.try_recv().is_ok());
        release.send(()).unwrap();
        app.run_until(LIMIT, |e| {
            e.document.dust.has_evaluation(source, replacement)
        });
        drop(probe);
        app.read(|e| {
            assert!(!e.document.dust.has_evaluation(source, old));
            assert_eq!(e.dust_repairs(source).unwrap().repairs().len(), 1);
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn document_or_application_retirement_keeps_running_gpu_resources_until_physical_completion() {
    for close_application in [false, true] {
        let (gate, armed, starts, release) = gated();
        let cache = Arc::new(crate::gpu::dust_screening::Cache::default());
        let weak = Arc::downgrade(&cache);
        let (mut e, scheduler) = fixture(gate, cache);
        automatic(&mut e);
        with_application(e, |app| {
            armed.store(true, Ordering::Release);
            start(app);
            app.pump();
            starts.recv_timeout(LIMIT).unwrap();
            if close_application {
                app.close_execution();
            }
            app.update(|e| {
                e.document = Document::default().into();
                e.dust_services = None;
            });
            if !close_application {
                app.pump();
            }
            assert!(weak.upgrade().is_some());
            app.read(|e| assert!(e.document.dust.pending_request().is_none()));
            release.send(()).unwrap();
        });
        let deadline = std::time::Instant::now() + LIMIT;
        while weak.upgrade().is_some() {
            assert!(
                std::time::Instant::now() < deadline,
                "retired repair kept GPU resources"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn source_visit_retirement_cancels_healing_without_promoting_old_evaluations_on_return() {
    let (gate, armed, starts, release) = gated();
    let (mut e, scheduler) = fixture(gate, Arc::default());
    automatic(&mut e);
    let source = e.current_source.unwrap();
    let path = e.document.source_path(source).unwrap();
    let signature = e.dust_signature().unwrap();
    let instruction = e.dust_repairs(source).unwrap().repairs()[0].clone();
    with_application(e, |app| {
        armed.store(true, Ordering::Release);
        start(app);
        app.pump();
        starts.recv_timeout(LIMIT).unwrap();
        let old_visit = app.read(|e| e.id);
        app.enter(|e, access| {
            e.load(path, access);
        });
        app.pump();
        app.read(|e| assert_ne!(e.id, old_visit));
        release.send(()).unwrap();
        app.run_until(LIMIT, |e| {
            !e.busy && e.document.dust.pending_request().is_none()
        });
        app.read(|e| {
            assert!(
                e.document
                    .dust
                    .evaluations(
                        source,
                        old_visit,
                        signature,
                        std::slice::from_ref(&instruction),
                    )
                    .is_empty()
            )
        });
    });
    scheduler.close_and_wait().unwrap();
}
