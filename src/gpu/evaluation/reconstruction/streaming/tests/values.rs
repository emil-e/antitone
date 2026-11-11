use super::*;

#[test]
fn bounded_operation_partitioning_does_not_change_the_completed_reconstruction_identity() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 8 << 20,
        },
    );
    let scope = context.test_materializations(true);
    let (input, reads) = fixture([128, 65], vec![[2.5, -1.25, 10_000., 1.]; 128 * 65]);
    let map = Mapping {
        start: [0.; 2],
        axes: [[128., 0.], [0., 65.]],
    };
    let region = Rect::new([0, 0], [1, 1]);
    let domain = output(map, region);
    let root = || reconstruct_mapped(input.clone(), domain, map).unwrap();
    let gate = crate::gpu::submission::SubmissionGate::default();
    let submissions = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = submissions.clone();
    gate.observe_submissions(Arc::new(move || {
        observed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }));
    let acquire = |budget| {
        pollster::block_on(
            context
                .materialize(
                    &queue,
                    &gate,
                    &scope,
                    context
                        .request(
                            root(),
                            Region {
                                bounds: region,
                                grid: domain.grid,
                            },
                            Budget {
                                tile: [1, 1],
                                scratch_bytes: budget,
                            },
                        )
                        .unwrap(),
                    ImageFormat::Rgba32Float,
                    ImageBacking::Buffer,
                )
                .unwrap(),
        )
        .unwrap()
    };
    let first = acquire(32 << 10);
    let acquired = reads.lock().unwrap().len();
    assert!(
        acquired > 1,
        "the first calculation must really use bounded source contributions"
    );
    assert_eq!(
        super::super::super::super::execute::read(&device, &queue, first.as_buffer().unwrap()),
        vec![[2.5, -1.25, 10_000., 9.]]
    );
    submissions.store(0, std::sync::atomic::Ordering::Relaxed);
    let second = acquire(128 << 10);
    assert_eq!(
        first.identity, second.identity,
        "execution budget is not calculation identity"
    );
    assert_eq!(first.descriptor, second.descriptor);
    assert_eq!(
        reads.lock().unwrap().len(),
        acquired,
        "the changed budget must not reacquire source inputs"
    );
    assert_eq!(
        submissions.load(std::sync::atomic::Ordering::Relaxed),
        0,
        "different execution partitions must consume completed pixels without GPU recomputation"
    );
    assert_eq!(
        super::super::super::super::execute::read(&device, &queue, second.as_buffer().unwrap()),
        vec![[2.5, -1.25, 10_000., 9.]]
    );
    assert_eq!(
        reads.lock().unwrap().len(),
        acquired,
        "cache reuse must not repeat original source acquisition"
    );
}
