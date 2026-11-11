use super::graph::Graph;
use super::*;

#[test]
fn live_completed_units_are_consumed_without_rebuilding_their_constituent_graphs() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let gate = crate::gpu::submission::SubmissionGate::default();
    let plate = Plate::new();
    let area = CellRegion::new([0, 6], [8, 4]);
    let plans = Arc::new(AtomicUsize::new(0));
    let root = Value::from_operation(
        plate.domain,
        [source(Arc::new(plate.clone()))],
        Graph {
            steps: 1,
            plans: plans.clone(),
        },
    );
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 8 << 20,
        },
    );
    let scope = context.test_materializations(true);
    let acquire = || {
        pollster::block_on(
            context
                .materialize(
                    &queue,
                    &gate,
                    &scope,
                    context
                        .request(
                            root.clone(),
                            Region {
                                bounds: area,
                                grid: plate.domain.grid,
                            },
                            budget(area),
                        )
                        .unwrap(),
                    ImageFormat::Rgba32Float,
                    ImageBacking::Buffer,
                )
                .unwrap(),
        )
        .unwrap()
    };
    let first = acquire();
    let held = scope
        .ready::<ImageLayout>(root.identity().unwrap())
        .into_iter()
        .filter(|image| image.descriptor.region.width == 2 && image.descriptor.region.height == 1)
        .collect::<Vec<_>>();
    assert_eq!(
        held.len(),
        16,
        "the control must retain actual constituent units, not just a whole-frame cache hit"
    );
    assert!(
        plans.load(Ordering::Relaxed) > 0,
        "cold unit execution must have built its graph"
    );
    scope.release_optional(root.identity().unwrap());
    drop(first);
    plans.store(0, Ordering::Relaxed);
    let second = acquire();
    assert_eq!(
        plans.load(Ordering::Relaxed),
        0,
        "discover completed unit values before building replacement work"
    );
    let actual = super::super::super::execute::read(&device, &queue, second.as_buffer().unwrap());
    assert_eq!(actual.len(), 32);
    for (i, pixel) in actual.iter().enumerate() {
        let p = [area.x + (i % 8) as i32, area.y + (i / 8) as i32];
        let base = plate.pixel(p);
        assert_eq!(
            &pixel[..3],
            &[2. * base[0] + 1., 2. * base[1] - 2., 2. * base[2] + 3.]
        );
        assert_eq!(pixel[3].to_bits(), base[3].to_bits());
    }
    drop(held);
}
