use super::*;
use crate::editor::{Document, document_scope::DocumentScope};
use std::path::Path;

#[cfg(feature = "ocio-reference")]
mod exposure;

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
    let expected = document
        .project
        .source(document.project.order[0])
        .unwrap()
        .clone();
    let wb = document
        .project
        .development()
        .map(|development| development.fixed_wb);
    let document: DocumentScope = document.into();
    let file = document.file_key(&path);
    let request = Request {
        reference: Arc::default(),
        input: document.project.input,
        path,
        wb,
        expected: Some(expected),
        file,
        cache: Some(document.cache.clone()),
        correction: None,
    };
    (document, request)
}

#[test]
fn acquisition_preserves_source_identity_precision_and_pixels_without_editor_or_gpu() {
    let (document, request) = request("patches.dng");
    let scheduler = source_loading::SourceLoader::new();
    let developed = request.load(&scheduler, &|| false).unwrap();
    let context = developed.context;
    assert_eq!(document.cache.charges().retained, 0);
    assert_eq!(
        developed.record.development,
        document.project.sources[0].development
    );
    assert_eq!(
        context.source.input_floor(),
        developed.record.development.input_floor
    );
    let mut scan = context.scan().unwrap();
    assert!(
        Arc::ptr_eq(scan.source(), &context.source),
        "a job context must lease, not copy, the source"
    );
    document.cache.close();
    let measured = scan
        .reference(crate::raw::Rect {
            x: 24,
            y: 24,
            width: 16,
            height: 16,
        })
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
    let (document, mut request) = request("patches.dng");
    let accepted = document.project.clone();
    request.expected.as_mut().unwrap().development.camera_model = "different camera".into();
    let scheduler = source_loading::SourceLoader::new();
    assert!(request.load(&scheduler, &|| false).is_err());
    assert_eq!(document.project, accepted);
    assert_eq!(document.cache.charges().retained, 0);
    scheduler.close_and_wait().unwrap();
}

#[test]
fn dropping_the_operation_owner_retires_native_demand_before_the_native_call_returns() {
    for close_store in [false, true] {
        let (document, request) = request("patches.dng");
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
        if close_store {
            document.cache.close();
        }
        drop(owner);
        let result = returns
            .recv_timeout(Duration::from_secs(5))
            .expect("source waiter did not retire");
        assert!(matches!(result, Err(Error::UnusableSample(_))));
        assert_eq!(document.cache.charges().retained, 0);
        release.send(()).unwrap();
        waiter.join().unwrap();
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn operation_cancellation_prevents_native_start_but_optional_cache_closure_does_not() {
    for cancelled in [false, true] {
        let (document, request) = request("patches.dng");
        let entered = Arc::new(AtomicBool::new(false));
        let observed = entered.clone();
        let scheduler = source_loading::SourceLoader::new_observed(move |_, _, _| {
            observed.store(true, Ordering::Relaxed);
            Ok(())
        });
        document.cache.close();
        assert_eq!(request.load(&scheduler, &|| cancelled).is_err(), cancelled);
        scheduler.close_and_wait().unwrap();
        assert_eq!(entered.load(Ordering::Relaxed), !cancelled);
        assert_eq!(document.cache.charges().retained, 0);
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
