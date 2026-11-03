use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone)]
struct Pixels {
    domain: Domain,
    values: Arc<Vec<[f32; 4]>>,
    reads: Arc<AtomicUsize>,
}
impl Source for Pixels {
    fn domain(&self) -> Domain {
        self.domain
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(self.values.clone()))
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let mut values = Vec::new();
        for y in region.y..region.bottom() {
            let start = self.domain.finite().unwrap().index([region.x, y]);
            values.extend_from_slice(&self.values[start..start + region.width as usize]);
        }
        Ok(values)
    }
}
fn fixture() -> Pixels {
    let mut grid = Grid::axis_aligned([0.; 2], [1.; 2]);
    grid.coverage = 8;
    Pixels {
        domain: Domain {
            bounds: Some([4, 5, 8, 6].into()),
            grid,
            meaning: Meaning::Neutral,
        },
        values: Arc::new(
            (0..6)
                .flat_map(|y| {
                    (0..8).map(move |x| {
                        [
                            x as f32 - 4.,
                            2. * y as f32 - 7.,
                            x as f32 + 3. * y as f32,
                            if x == 0 && y == 0 {
                                9.
                            } else if x == 7 && y == 5 {
                                12.
                            } else {
                                8.
                            },
                        ]
                    })
                })
                .collect(),
        ),
        reads: Arc::new(AtomicUsize::new(0)),
    }
}
fn evaluation(context: &Context, image: Arc<ImageNode>, region: CellRegion) -> Evaluation {
    context
        .request(
            image.clone(),
            Region {
                bounds: region,
                grid: image.domain.grid,
            },
            Budget {
                tile: [2, 2],
                scratch_bytes: 1 << 20,
            },
        )
        .unwrap()
}

#[test]
fn equivalent_awaiters_produce_once_and_larger_stored_regions_supply_smaller_requests() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 4096,
        },
    );
    let scope = context.test_materializations(true);
    let pixels = fixture();
    let one = source(Arc::new(pixels.clone()));
    let two = source(Arc::new(pixels.clone()));
    let gate = crate::gpu::submission::SubmissionGate::default();
    let first = context
        .materialize(
            &queue,
            &gate,
            &scope,
            evaluation(&context, one, pixels.domain.finite().unwrap()),
            ImageFormat::Rgba32Float,
            ImageBacking::Buffer,
        )
        .unwrap();
    let second = context
        .materialize(
            &queue,
            &gate,
            &scope,
            evaluation(&context, two.clone(), pixels.domain.finite().unwrap()),
            ImageFormat::Rgba32Float,
            ImageBacking::Buffer,
        )
        .unwrap();
    let (first, second) = pollster::block_on(futures_util::future::join(first, second));
    let first = first.unwrap();
    let second = second.unwrap();
    assert!(Arc::ptr_eq(&first.image, &second.image));
    assert_eq!(pixels.reads.load(Ordering::SeqCst), 1);
    assert_eq!(
        execute::read(&device, &queue, first.as_buffer().unwrap()),
        *pixels.values
    );
    let small = [6, 7, 2, 2].into();
    let subset = pollster::block_on(
        context
            .materialize(
                &queue,
                &gate,
                &scope,
                evaluation(&context, two, small),
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        subset.descriptor.region,
        pixels.domain.finite().unwrap(),
        "addressing describes the actual stored region, not the smaller demand"
    );
    assert!(Arc::ptr_eq(&first.image, &subset.image));
    assert_eq!(pixels.reads.load(Ordering::SeqCst), 1);
    scope.clear();
    assert_eq!(context.retained_bytes(), 0);
    assert_eq!(
        execute::read(&device, &queue, first.as_buffer().unwrap()),
        *pixels.values
    );
}

#[test]
fn awaiting_a_composed_image_submits_its_gpu_work_without_a_consumer_submission() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.test_materializations(false);
    let data = fixture();
    let mut grid = Grid::axis_aligned([4., 5.], [2., 2.]);
    grid.coverage = 8;
    let output = Domain {
        bounds: Some([0, 0, 4, 3].into()),
        grid,
        meaning: Meaning::Neutral,
    };
    let root =
        super::super::super::reconstruction::reconstruct(source(Arc::new(data)), output).unwrap();
    let key = root.identity().unwrap().clone();
    let gate = crate::gpu::submission::SubmissionGate::default();
    let submits = Arc::new(AtomicUsize::new(0));
    let observed = submits.clone();
    gate.observe_submissions(Arc::new(move || {
        observed.fetch_add(1, Ordering::SeqCst);
    }));
    let image = pollster::block_on(
        context
            .materialize(
                &queue,
                &gate,
                &scope,
                evaluation(&context, root, output.finite().unwrap()),
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap();
    assert!(
        submits.load(Ordering::SeqCst) > 0,
        "the evaluator, not a later consumer, submits production"
    );
    let values = execute::read(&device, &queue, image.as_buffer().unwrap());
    for y in 0..3 {
        for x in 0..4 {
            let expected = [
                2. * x as f32 + 0.5 - 4.,
                4. * y as f32 + 1. - 7.,
                2. * x as f32 + 6. * y as f32 + 2.,
            ];
            let actual = values[y * 4 + x];
            for c in 0..3 {
                assert!(
                    (actual[c] - expected[c]).abs() < 1e-6,
                    "mean of four original linear samples ({x},{y}) channel {c}"
                );
            }
            assert_eq!(
                actual[3],
                if x == 0 && y == 0 {
                    9.
                } else if x == 3 && y == 2 {
                    12.
                } else {
                    8.
                }
            );
        }
    }
    assert_eq!(context.retained_bytes(), 0);
    let active = pollster::block_on(
        scope
            .find::<ImageLayout>(&key, |stored| stored.region == output.finite().unwrap())
            .unwrap(),
    )
    .unwrap();
    assert!(
        Arc::ptr_eq(&active.image, &image.image),
        "zero retention still allows sharing a live input"
    );
}
