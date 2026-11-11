use super::preparation_tests::{detect, observed, request};
use super::*;
use std::sync::atomic::Ordering;

#[test]
fn async_review_uses_finished_results_without_source_dependencies() {
    let (mut request, _directory, device, queue) = request();
    let (scheduler, count) = observed();
    detect(&mut request, &scheduler, &device, &queue);
    let before = request.document.project.clone();
    for _ in 0..2 {
        let approved = futures_lite::future::block_on(prepare_review(
            request.clone(),
            Cancellation::default(),
            |_, _| {},
        ))
        .unwrap()
        .unwrap();
        assert_eq!(approved.entries.len(), 2);
        assert!(approved.entries.iter().all(|entry| entry.areas.ready()));
    }
    assert_eq!(count.load(Ordering::Relaxed), 2);
    assert_eq!(request.document.project, before);
    scheduler.close_and_wait().unwrap();
}

#[test]
fn async_assembly_cancellation_keeps_finished_shared_results_and_accepted_state() {
    let (mut request, _directory, device, queue) = request();
    let (scheduler, count) = observed();
    detect(&mut request, &scheduler, &device, &queue);
    let before = request.document.project.clone();
    let ready = request.document.spatial_metadata.clone();
    let cancelled = Cancellation::default();
    let control = cancelled.clone();
    let result = futures_lite::future::block_on(prepare_review(
        request.clone(),
        cancelled,
        move |index, _| {
            if index == 1 {
                control.cancel();
            }
        },
    ))
    .unwrap();
    assert!(result.is_none());
    assert_eq!(count.load(Ordering::Relaxed), 2);
    assert_eq!(request.document.project, before);
    assert_eq!(request.document.spatial_metadata, ready);
    scheduler.close_and_wait().unwrap();
}
