use super::*;
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

fn request(name: &str) -> (Document, Request) {
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
    let cache = Cache::new(&path, Some(&expected), &document);
    let request = Request {
        path,
        wb,
        expected: Some(expected),
        cache: Some(cache),
        correction: None,
    };
    (document, request)
}

#[test]
fn acquisition_preserves_source_identity_precision_and_pixels_without_editor_or_gpu() {
    let (document, request) = request("patches.dng");
    let scheduler = development::Scheduler::new();
    let developed = request.load(&scheduler, &|| false).unwrap();
    let context = developed.context;
    assert_eq!(document.cache().charges().retained, 0);
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
    document.cache().close();
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
    let scheduler = development::Scheduler::new();
    assert!(request.load(&scheduler, &|| false).is_err());
    assert_eq!(document.project, accepted);
    assert_eq!(document.cache().charges().retained, 0);
    scheduler.close_and_wait().unwrap();
}

#[test]
fn native_wait_retires_on_cancellation_or_store_close_before_the_native_call_returns() {
    for close_store in [false, true] {
        let (document, request) = request("patches.dng");
        let (entered, starts) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let resume = Mutex::new(resume);
        let scheduler = development::Scheduler::new_observed(move |_, phase, _| {
            if matches!(phase, development::NativePhase::UnpackBegin) {
                entered.send(()).unwrap();
                resume
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap();
            }
            Ok(())
        });
        let retired = Arc::new(AtomicBool::new(false));
        let cancel = retired.clone();
        let sources = scheduler.clone();
        let (returned, returns) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            returned
                .send(request.load(&sources, &|| cancel.load(Ordering::Relaxed)))
                .unwrap();
        });
        starts.recv_timeout(Duration::from_secs(5)).unwrap();
        if close_store {
            document.cache().close();
        } else {
            retired.store(true, Ordering::Relaxed);
        }
        let result = returns
            .recv_timeout(Duration::from_secs(5))
            .expect("source waiter did not retire");
        assert!(matches!(result, Err(Error::UnusableSample(_))));
        assert_eq!(document.cache().charges().retained, 0);
        release.send(()).unwrap();
        waiter.join().unwrap();
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn a_closed_or_cancelled_request_cannot_start_native_work() {
    for close_store in [false, true] {
        let (document, request) = request("patches.dng");
        let entered = Arc::new(AtomicBool::new(false));
        let observed = entered.clone();
        let scheduler = development::Scheduler::new_observed(move |_, _, _| {
            observed.store(true, Ordering::Relaxed);
            Ok(())
        });
        if close_store {
            document.cache().close();
        }
        assert!(request.load(&scheduler, &|| !close_store).is_err());
        scheduler.close_and_wait().unwrap();
        assert!(!entered.load(Ordering::Relaxed));
    }
}
