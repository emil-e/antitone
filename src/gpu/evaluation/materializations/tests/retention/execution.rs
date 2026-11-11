use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Constant(Arc<AtomicUsize>);
impl Source for Constant {
    fn domain(&self) -> Domain {
        let region = [0, 0, 2, 2].into();
        Domain {
            bounds: Some(region),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::Neutral,
        }
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("live source execution"))
    }
    fn read(&self, r: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(vec![
            [2., -0.25, 3., 9.];
            r.width as usize * r.height as usize
        ])
    }
}
#[test]
fn single_and_batched_materializations_preserve_independent_retention_choices() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    for (batch, optional) in [(false, false), (false, true), (true, false), (true, true)] {
        let context = Context::new(
            &device,
            CachePolicy {
                generated_bytes: 128,
            },
        );
        let scope = context.materializations();
        scope.set_test_pressure(crate::sys::MemoryPressure::Normal);
        let reads = Arc::new(AtomicUsize::new(0));
        let root = source(Arc::new(Constant(reads.clone())));
        let key = root.identity().unwrap().clone();
        let request = context
            .request(
                root.clone(),
                Region {
                    bounds: root.domain().finite().unwrap(),
                    grid: root.domain().grid,
                },
                Budget {
                    tile: [2, 2],
                    scratch_bytes: 4096,
                },
            )
            .unwrap();
        let live = request.clone().without_retention();
        let images = if batch {
            pollster::block_on(futures_util::future::try_join_all(
                context
                    .materialize_many(
                        &queue,
                        &Default::default(),
                        &scope,
                        vec![
                            live.clone(),
                            if optional {
                                request.clone()
                            } else {
                                live.clone()
                            },
                        ],
                    )
                    .unwrap(),
            ))
            .unwrap()
        } else {
            vec![
                pollster::block_on(
                    context
                        .materialize(
                            &queue,
                            &Default::default(),
                            &scope,
                            live,
                            ImageFormat::Rgba32Float,
                            ImageBacking::Buffer,
                        )
                        .unwrap(),
                )
                .unwrap(),
            ]
        };
        let mut images = images;
        if optional && !batch {
            images.push(
                pollster::block_on(
                    context
                        .materialize(
                            &queue,
                            &Default::default(),
                            &scope,
                            request,
                            ImageFormat::Rgba32Float,
                            ImageBacking::Buffer,
                        )
                        .unwrap(),
                )
                .unwrap(),
            );
        }
        assert_eq!(reads.load(Ordering::Relaxed), 1);
        assert_eq!(context.retained_bytes(), if optional { 64 } else { 0 });
        if images.len() > 1 {
            assert!(Arc::ptr_eq(&images[0].image, &images[1].image));
        }
        let joined = scope.find_ready::<ImageLayout>(&key, |_| true).unwrap();
        drop(images);
        assert_eq!(
            execute::read(&device, &queue, joined.as_buffer().unwrap()),
            [[2., -0.25, 3., 9.]; 4]
        );
        drop(joined);
        assert_eq!(
            scope.find_ready::<ImageLayout>(&key, |_| true).is_some(),
            optional,
            "only an independently selected optional request keeps discovery after consumers retire"
        );
    }
}
#[test]
fn inline_execution_inherits_a_live_roots_reservation() {
    let (device, _, _) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 128,
        },
    );
    let scope = context.materializations();
    scope.set_test_pressure(crate::sys::MemoryPressure::Normal);
    let region = [0, 0, 2, 2].into();
    let layout = Arc::new(ImageLayout {
        domain: Domain {
            bounds: Some(region),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::Neutral,
        },
        region,
        format: ImageFormat::Rgba32Float,
        backing: ImageBacking::Buffer,
    });
    let budget = Budget {
        tile: [2, 2],
        scratch_bytes: 4096,
    };
    let key = RetainedKey::new("queued live root");
    let queued = scope.obtain_for(
        key.clone(),
        layout.clone(),
        64,
        RequestPolicy {
            budget: Some(budget),
            retain: false,
        },
        || async {
            panic!("inline production must fulfil the calculation before queued work starts")
        },
    );
    let (future, producer) = scope.claim_unit(key.clone(), layout, budget);
    producer
        .unwrap()
        .complete(Ok(ImageValue::new(buffer(&device, 5.))));
    let completed = pollster::block_on(future).unwrap();
    assert_eq!(
        context.retained_bytes(),
        0,
        "stealing a queued root must not promote it to optional retention"
    );
    drop((queued, completed));
    assert!(scope.find_ready::<ImageLayout>(&key, |_| true).is_none());
}
