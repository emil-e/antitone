use super::*;
use crate::editor::{Document, document_scope::DocumentScope};
use std::path::Path;

mod decoder_version;
#[cfg(feature = "ocio-reference")]
mod exposure;
#[cfg(target_os = "macos")]
mod raw_formats;
#[cfg(target_os = "macos")]
mod xtrans;

use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/raw")
        .join(name)
}

fn request(name: &str) -> (DocumentScope, Request) {
    let path = fixture(name);
    let mut document = Document::default();
    document.import(std::slice::from_ref(&path)).unwrap();
    let document: DocumentScope = document.into();
    let request = Request {
        source: document.source_input(document.project.order[0]).unwrap(),
        sources: None,
        correction: Default::default(),
    };
    (document, request)
}

#[test]
fn source_scope_cancellation_retires_the_continuation_before_publication() {
    for polled in [false, true] {
        for reply_ready in [false, true] {
            for retirement in 0..4 {
                let editor = crate::editor::tests::fixture();
                let published = Arc::new(AtomicBool::new(false));
                let cancelled = crate::cancellation::Cancellation::default();
                let (reply, result) = futures_channel::oneshot::channel();
                crate::application::test_support::with_application(editor, |app| {
                    app.enter(|editor, cx| {
                        let published = published.clone();
                        let retire = cancelled.cancel_on_drop();
                        editor.document.visit.source.task = cx.spawn(move |cx| async move {
                            let _retire = retire;
                            let result = result.await.expect("source reply retained");
                            published.store(true, Ordering::SeqCst);
                            cx.update(|editor| editor.source_result(result));
                        });
                    });
                    if polled {
                        app.pump();
                    }
                    let mut reply = Some(reply);
                    if reply_ready {
                        assert!(
                            reply
                                .take()
                                .unwrap()
                                .send(Err("completed source request".into()))
                                .is_ok()
                        );
                    }
                    app.update(|editor| match retirement {
                        0 => {}
                        1 => {
                            editor.replace_photo_scope(editor.document.visit.current_source);
                        }
                        2 => {
                            editor.reinterpret_photo();
                        }
                        3 => editor.new_project(),
                        _ => unreachable!(),
                    });
                    if let Some(reply) = reply {
                        let _ = reply.send(Err("completed source request".into()));
                    }
                    app.pump();
                    assert_eq!(
                        published.load(Ordering::SeqCst),
                        retirement == 0,
                        "polled={polled}, reply_ready={reply_ready}, retirement={retirement}"
                    );
                    assert!(cancelled.is_cancelled());
                    app.read(|editor| {
                        assert!(!editor.document.visit.source.pending());
                        if retirement == 0 {
                            assert_eq!(editor.status, "completed source request");
                        } else {
                            assert_ne!(editor.status, "completed source request");
                        }
                    });
                });
            }
        }
    }
}

#[test]
fn acquisition_preserves_source_identity_and_pixels_without_editor_or_gpu() {
    let (document, mut request) = request("patches.dng");
    let scheduler = source_loading::SourceLoader::new();
    request.sources = Some(scheduler.provider(Some(document.cache.clone())));
    let developed = request.load(&scheduler, &|| false).unwrap();
    let context = developed.context;
    assert_eq!(document.cache.charges().retained, 0);
    assert_eq!(
        developed.record.development,
        document.project.sources[0].development
    );
    let scan = context.source.clone();
    assert!(
        Arc::ptr_eq(&scan, &context.source),
        "a job context must lease, not copy, the source"
    );
    let cache = Arc::downgrade(&document.cache);
    drop(document);
    assert!(cache.upgrade().is_none());
    let measured = crate::sampling::camera::reference(
        &scan,
        None,
        crate::raw::Rect {
            x: 24,
            y: 24,
            width: 16,
            height: 16,
        },
    )
    .unwrap();
    // Authored 12-bit patch: black 64, white 4095, film-base code 3064.
    assert!(
        (measured - glam::DVec3::splat(3000. / 4031.))
            .abs()
            .max_element()
            <= 2. / 65535.
    );
    scheduler.close_and_wait().unwrap();
}

#[test]
fn incompatible_requests_reject_without_rewriting_document_authority() {
    for conflicting_wb in [false, true] {
        let (document, mut request) = request("patches.dng");
        let accepted = document.project.clone();
        let mut record = document.project.sources[0].clone();
        if conflicting_wb {
            record.development.decode.white_balance = [2., 1., 1., 1.];
        } else {
            record.development.camera_model = "different camera".into();
        }
        request.source = crate::source::Input::photo(
            document.source_path(record.id).unwrap(),
            document.file_key(&document.source_path(record.id).unwrap()),
            document.project.input,
            Arc::default(),
            crate::source::input::Photograph {
                record: record.clone(),
                development: record.development.clone().into(),
            },
        );
        let scheduler = source_loading::SourceLoader::new();
        assert!(request.load(&scheduler, &|| false).is_err());
        assert_eq!(document.project, accepted);
        assert_eq!(document.cache.charges().retained, 0);
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn dropping_the_operation_owner_retires_native_demand_before_the_native_call_returns() {
    for retire_document in [false, true] {
        let (document, mut request) = request("patches.dng");
        let cache = Arc::downgrade(&document.cache);
        let mut document = Some(document);
        let (entered, starts) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let resume = Mutex::new(resume);
        let scheduler = source_loading::SourceLoader::new_observed(move |_, phase, _| {
            if matches!(phase, source_loading::NativePhase::UnpackBegin) {
                entered.send(()).unwrap();
                resume
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap();
            }
            Ok(())
        });
        request.sources = Some(scheduler.provider(Some(document.as_ref().unwrap().cache.clone())));
        let cancel = crate::cancellation::Cancellation::default();
        let owner = cancel.cancel_on_drop();
        let sources = scheduler.clone();
        let (returned, returns) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            returned
                .send(request.load(&sources, &|| cancel.is_cancelled()))
                .unwrap();
        });
        starts.recv_timeout(Duration::from_secs(5)).unwrap();
        if retire_document {
            drop(document.take());
        }
        drop(owner);
        let result = returns
            .recv_timeout(Duration::from_secs(5))
            .expect("source waiter did not retire");
        assert!(matches!(result, Err(Error::UnusableSample(_))));
        assert_eq!(cache.upgrade().is_none(), retire_document);
        release.send(()).unwrap();
        waiter.join().unwrap();
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn cacheless_acquisition_still_obeys_operation_cancellation() {
    for cancelled in [false, true] {
        let (document, mut request) = request("patches.dng");
        request.sources = None;
        let entered = Arc::new(AtomicBool::new(false));
        let observed = entered.clone();
        let scheduler = source_loading::SourceLoader::new_observed(move |_, _, _| {
            observed.store(true, Ordering::Relaxed);
            Ok(())
        });
        let cache = Arc::downgrade(&document.cache);
        drop(document);
        assert!(cache.upgrade().is_none());
        assert_eq!(request.load(&scheduler, &|| cancelled).is_err(), cancelled);
        scheduler.close_and_wait().unwrap();
        assert_eq!(entered.load(Ordering::Relaxed), !cancelled);
    }
}

#[test]
fn async_acquisition_retires_its_waiter_when_the_operation_owner_drops() {
    let (_document, request) = request("patches.dng");
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Mutex::new(resume);
    let loader = source_loading::SourceLoader::new_observed(move |_, phase, _| {
        if matches!(phase, source_loading::NativePhase::UnpackBegin) {
            entered.send(()).unwrap();
            resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
        }
        Ok(())
    });
    let mut services = crate::editor::tests::authored_fixture(&[])
        .source_services
        .take()
        .unwrap();
    services.developments = loader.clone();
    let cancelled = crate::cancellation::Cancellation::default();
    let owner = cancelled.cancel_on_drop();
    let (returned, returns) = mpsc::channel();
    let waiter = std::thread::spawn(move || {
        let result = futures_lite::future::block_on(services.acquire(request, cancelled));
        returned.send(result).unwrap();
    });
    starts.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(owner);
    assert!(
        returns
            .recv_timeout(Duration::from_secs(5))
            .expect("async waiter must wake before native completion")
            .is_err()
    );
    assert_eq!(loader.waiter_count(), 0);
    release.send(()).unwrap();
    waiter.join().unwrap();
    loader.close_and_wait().unwrap();
}
