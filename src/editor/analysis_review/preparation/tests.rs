use super::super::preparation_tests::{observed, request};
use super::*;
use crate::editor::analysis_services::Services;

#[test]
fn awaited_native_stages_reuse_exact_scratch_and_fall_back_without_changing_areas() {
    let (request, _directory, device, queue) = request();
    let before = request.document.project.clone();
    let (scheduler, count) = observed();
    let services = Services {
        exposures: Arc::default(),
        sources: scheduler.provider(None),
        device,
        queue,
        renderers: Arc::new(crate::renderer::RendererCache::new(Default::default())),
    };
    let run = |measurements| {
        futures_lite::future::block_on(prepare_staged(
            request.clone(),
            services.clone(),
            Cancellation::default(),
            |_, _| {},
            measurements,
        ))
        .unwrap()
        .unwrap()
    };
    let cached = run(Default::default());
    // Each RAW supplies coarse evidence and exact fine scratch once; unavailable or
    // full scratch requires a second development per photo, not retained native leases.
    assert_eq!(count.load(Ordering::Relaxed), 2);
    let unavailable = run(measurements::Measurements::unavailable());
    assert_eq!(count.load(Ordering::Relaxed), 6);
    let full = run(measurements::Measurements::full());
    assert_eq!(count.load(Ordering::Relaxed), 10);
    assert_eq!(cached.entries, unavailable.entries);
    assert_eq!(cached.entries, full.entries);
    assert_eq!(cached.files, unavailable.files);
    assert_eq!(request.document.project, before);

    scheduler.close_and_wait().unwrap();
}

#[test]
fn async_roll_barrier_cancellation_does_not_reacquire_or_return_partial_support() {
    let (request, _directory, device, queue) = request();
    let before = request.document.project.clone();
    let (scheduler, count) = observed();
    let cancelled = Cancellation::default();
    let control = cancelled.clone();
    let result = futures_lite::future::block_on(prepare(
        request.clone(),
        Services {
            exposures: Arc::default(),
            sources: scheduler.provider(None),
            device,
            queue,
            renderers: Arc::new(crate::renderer::RendererCache::new(Default::default())),
        },
        cancelled,
        move |index, total| {
            if index == total / 2 {
                control.cancel();
            }
        },
    ))
    .unwrap();
    assert!(result.is_none());
    assert_eq!(count.load(Ordering::Relaxed), 2);
    assert_eq!(request.document.project, before);
    scheduler.close_and_wait().unwrap();
}
