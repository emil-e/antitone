use super::*;

#[test]
fn traversal_hints_do_not_change_the_completed_reconstruction_identity() {
    let (device, queue, _) = crate::photographic::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 8 << 20,
        },
    );
    let scope = context.test_materializations(true);
    let (input, reads) = fixture([512, 257], vec![[2.5, -1.25, 10_000., 1.]; 512 * 257]);
    let map = Mapping {
        start: [0.; 2],
        axes: [[512., 0.], [0., 257.]],
    };
    let region = Rect::new([0, 0], [1, 1]);
    let domain = output(map, region);
    let root = || reconstruct_mapped(input.clone(), domain, map).unwrap();
    let gate = queue.clone();
    let submissions = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = submissions.clone();
    gate.observe_submissions(Arc::new(move || {
        observed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }));
    let acquire = |tile| {
        pollster::block_on(
            context
                .materialize(
                    &queue,
                    &scope,
                    context
                        .request(
                            root(),
                            Region {
                                bounds: region,
                                grid: domain.grid,
                            },
                            Tiling { tile },
                        )
                        .unwrap(),
                    ImageFormat::Rgba32Float,
                    ImageBacking::Buffer,
                )
                .unwrap(),
        )
        .unwrap()
    };
    let first = acquire([1, 1]);
    let acquired = reads.lock().unwrap().len();
    assert!(
        acquired > 1,
        "the first calculation must really use bounded source contributions"
    );
    assert_eq!(
        crate::evaluation::read_buffer(&device, &queue, first.as_buffer().unwrap()),
        vec![[2.5, -1.25, 10_000., 9.]]
    );
    submissions.store(0, std::sync::atomic::Ordering::Relaxed);
    let second = acquire([256, 256]);
    assert_eq!(
        first.identity, second.identity,
        "traversal shape is not calculation identity"
    );
    assert_eq!(first.descriptor, second.descriptor);
    assert_eq!(
        reads.lock().unwrap().len(),
        acquired,
        "the changed traversal hint must not reacquire source inputs"
    );
    assert_eq!(
        submissions.load(std::sync::atomic::Ordering::Relaxed),
        0,
        "different execution partitions must consume completed pixels without GPU recomputation"
    );
    assert_eq!(
        crate::evaluation::read_buffer(&device, &queue, second.as_buffer().unwrap()),
        vec![[2.5, -1.25, 10_000., 9.]]
    );
    assert_eq!(
        reads.lock().unwrap().len(),
        acquired,
        "cache reuse must not repeat original source acquisition"
    );
}
