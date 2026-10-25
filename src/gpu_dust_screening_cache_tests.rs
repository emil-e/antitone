use super::*;

#[test]
fn compiled_screening_is_reused_across_jobs_and_a_panicking_job_can_discard_it_safely() {
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
    let submissions = SubmissionGate::default();
    let cache = Cache::default();
    cache.with(&device, &queue, &submissions, |screening| {
        screening.initialize().unwrap()
    });
    let compiled = cache.0.lock().unwrap().as_ref().unwrap().clone();
    cache.with(&device, &queue, &submissions, |screening| {
        assert_eq!(screening.pipeline.as_ref(), Some(&compiled));
        screening.initialize().unwrap();
        assert_eq!(screening.pipeline.as_ref(), Some(&compiled));
    });
    assert_eq!(cache.0.lock().unwrap().as_ref(), Some(&compiled));
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cache.with(&device, &queue, &submissions, |_| {
            panic!("screening job failed")
        });
    }));
    assert!(failure.is_err());
    cache.with(&device, &queue, &submissions, |screening| {
        screening.initialize().unwrap();
        assert!(screening.pipeline.is_some());
    });
}
