use super::*;
use crate::application::test_support::with_application;
use std::{
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

#[test]
fn cold_positive_captures_resolved_starting_exposure_before_authoring_its_graph() {
    let file = crate::raw::fixture::bayer_dng();
    let mut document = Document::default();
    document.project.input = crate::processing::InputSettings::Digital;
    document.import(&[file.path().to_owned()]).unwrap();
    let id = document.project.order[0];
    let source = document.project.source(id).unwrap().clone();
    let scan = Scan::open(file.path(), Some(source.development.decode.fixed_wb)).unwrap();
    let camera = Arc::new(crate::scan::CameraPixels::from_scan(&scan).unwrap());
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let loader = source_loading::SourceLoader::new();
    let thumbnails = Thumbnails::with_developments(
        device.clone(),
        queue.clone(),
        Arc::default(),
        None,
        loader.clone(),
    );
    let Services::Production(worker) = thumbnails.services.as_ref().unwrap() else {
        unreachable!()
    };
    for ev in [0., -1.] {
        let job = super::super::tests::service_job(
            &thumbnails,
            super::super::tests::key(&document, id),
            file.path().to_owned(),
            source.clone(),
            None,
            true,
        );
        assert!(
            job.key
                .recipe
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap()
                .neutral
                .reference
                .is_none()
        );
        let reference = crate::exposure_reference::ReferenceExposure::Matched(
            crate::exposure_reference::StartingExposure {
                ev,
                samples: 1024,
                spread: 0.,
            },
        );
        job.reference.set(reference.clone()).unwrap();
        let proxy = crate::scan::CameraProxy::new(camera.clone(), Some(reference.clone()));
        let product = worker
            .positive(&job, Some(&proxy), &mut ThumbnailMetrics::default())
            .unwrap()
            .unwrap();
        let (native, view) = product.positive.as_ref().unwrap();
        let crate::gpu::photograph::request::PhotographRequest::Recipe(captured) = &view.capture
        else {
            panic!("photographic thumbnail must retain its resolved recipe")
        };
        assert_eq!(captured.neutral.reference.as_ref(), Some(&reference));
        let program =
            crate::gpu::photograph::PhotographGraph::input_transform(&native.definition).unwrap();
        assert_eq!(program.matrix(), scan.camera_to_ap1 * ev.exp2());
    }
    loader.close_and_wait().unwrap();
}

#[test]
fn retiring_the_document_releases_retention_while_its_physical_thumbnail_job_finishes() {
    let file = crate::raw::fixture::bayer_dng();
    let mut document = Document::default();
    document.import(&[file.path().to_owned()]).unwrap();
    let document: super::super::super::document_scope::DocumentScope = document.into();
    let id = document.project.order[0];
    let source = document.project.source(id).unwrap().clone();
    let scan = Scan::open(file.path(), Some(source.development.decode.fixed_wb)).unwrap();
    let proxy = Arc::new(Proxy::from_scan(&scan).unwrap());
    let pixels = Arc::downgrade(&proxy);
    let cache = Arc::downgrade(&document.cache);
    let (device, queue) = pollster::block_on(async {
        crate::gpu::instance()
            .request_adapter(&Default::default())
            .await
            .unwrap()
            .request_device(&Default::default())
            .await
            .unwrap()
    });
    let loader = source_loading::SourceLoader::new();
    let mut thumbnails =
        Thumbnails::with_developments(device, queue, Arc::default(), None, loader.clone());
    thumbnails = thumbnails.with_sources(document.cache.clone());
    thumbnails.seed(&source, document.file_key(file.path()), proxy);
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Mutex::new(resume);
    let (done, finishes) = mpsc::channel();
    let Services::Production(worker) = thumbnails.services.as_mut().unwrap() else {
        unreachable!()
    };
    Arc::get_mut(worker).unwrap().observe = Some(Arc::new(move |_, beginning| {
        if beginning {
            entered.send(()).unwrap();
            resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
        } else {
            done.send(()).unwrap();
        }
    }));
    let mut editor = Editor::state();
    editor.document = document;
    editor.document.thumbnails = thumbnails.with_sources(editor.document.cache.clone());
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.request_thumbnail(
                super::super::tests::key(&editor.document, id),
                file.path().to_owned(),
                source,
                true,
                source_loading::Priority::Visible,
                cx,
            )
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while starts.try_recv().is_err() {
            app.pump();
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        app.update(|editor| editor.document = Document::default().into());
        let deadline = Instant::now() + Duration::from_secs(5);
        while cache.upgrade().is_some() {
            app.pump();
            assert!(
                Instant::now() < deadline,
                "physical thumbnail retained the document cache"
            );
            std::thread::yield_now();
        }
        assert!(
            pixels.upgrade().is_some(),
            "the running call still owns its needed pixels"
        );
        release.send(()).unwrap();
        finishes.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(pixels.upgrade().is_none());
        app.pump();
        app.read(|editor| {
            assert!(
                editor.document.state.project.order.is_empty()
                    && editor.document.thumbnails.cache.entries.is_empty()
            )
        });
    });
    loader.close_and_wait().unwrap();
}

#[test]
fn ready_thumbnail_processing_is_serial_and_visible_work_precedes_queued_eager_work() {
    let files: Vec<_> = (0..3).map(|_| crate::raw::fixture::bayer_dng()).collect();
    let mut document = Document::default();
    document
        .import(
            &files
                .iter()
                .map(|file| file.path().to_owned())
                .collect::<Vec<_>>(),
        )
        .unwrap();
    let document: super::super::super::document_scope::DocumentScope = document.into();
    let ids = document.project.order.clone();
    let (device, queue) = pollster::block_on(async {
        crate::gpu::instance()
            .request_adapter(&Default::default())
            .await
            .unwrap()
            .request_device(&Default::default())
            .await
            .unwrap()
    });
    let loader = source_loading::SourceLoader::new();
    let mut thumbnails =
        Thumbnails::with_developments(device, queue, Arc::default(), None, loader.clone());
    thumbnails = thumbnails.with_sources(document.cache.clone());
    thumbnails.sources.set_budget(64 << 20);
    for id in &ids {
        let source = document.project.source(*id).unwrap();
        let path = document.source_path(*id).unwrap();
        let scan = Scan::open(&path, Some(source.development.decode.fixed_wb)).unwrap();
        thumbnails.seed(
            source,
            document.file_key(&path),
            Arc::new(Proxy::from_scan(&scan).unwrap()),
        );
    }
    let first = ids[0];
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Mutex::new(resume);
    let active = Arc::new(AtomicUsize::new(0));
    let observed = active.clone();
    let worker = match thumbnails.services.as_mut().unwrap() {
        Services::Production(worker) => worker,
        _ => unreachable!(),
    };
    Arc::get_mut(worker).unwrap().observe = Some(Arc::new(move |id, beginning| {
        if beginning {
            assert_eq!(
                observed.fetch_add(1, Ordering::SeqCst),
                0,
                "thumbnail processing overlapped"
            );
            entered.send(id).unwrap();
            if id == first {
                resume
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(10))
                    .unwrap();
            }
        } else {
            assert_eq!(observed.fetch_sub(1, Ordering::SeqCst), 1);
        }
    }));
    let worker = worker.clone();
    let job = |id, priority| {
        let mut job = super::super::tests::service_job(
            &thumbnails,
            super::super::tests::key(&document, id),
            document.source_path(id).unwrap(),
            document.project.source(id).unwrap().clone(),
            None,
            true,
        );
        job.priority = priority;
        job
    };
    with_application(Vec::new(), |app| {
        let _first = super::super::tests::start_service_request(
            &thumbnails,
            job(ids[0], source_loading::Priority::Visible),
            &app.access(),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            app.pump();
            if let Ok(id) = starts.try_recv() {
                assert_eq!(id, ids[0]);
                break;
            }
            assert!(
                Instant::now() < deadline,
                "first thumbnail never entered processing"
            );
            std::thread::yield_now();
        }
        let _eager = super::super::tests::start_service_request(
            &thumbnails,
            job(ids[1], source_loading::Priority::Eager),
            &app.access(),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while worker.ready.lock().unwrap().pending.len() != 1 {
            app.pump();
            assert!(
                Instant::now() < deadline,
                "eager control never reached the ready queue first"
            );
            std::thread::yield_now();
        }
        let _visible = super::super::tests::start_service_request(
            &thumbnails,
            job(ids[2], source_loading::Priority::Visible),
            &app.access(),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while worker.ready.lock().unwrap().pending.len() != 2 {
            app.pump();
            assert!(
                Instant::now() < deadline,
                "other acquisitions could not become ready beside processing"
            );
            std::thread::yield_now();
        }
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(15), |outcomes| outcomes.len() == 3);
        assert_eq!(starts.try_iter().collect::<Vec<_>>(), vec![ids[2], ids[1]]);
        app.read(|outcomes| {
            assert!(outcomes.iter().all(|outcome| matches!(
                outcome,
                Ok(Outcome {
                    image: Ok(Some(_)),
                    proper: true,
                    ..
                })
            )))
        });
    });
    assert_eq!(active.load(Ordering::SeqCst), 0);
    loader.close_and_wait().unwrap();
}
