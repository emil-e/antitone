use super::*;
use std::sync::Mutex;

mod admission;
mod counts;
mod coverage;

struct Pixels {
    domain: Domain,
    values: Arc<Vec<[f32; 4]>>,
    reads: Arc<Mutex<Vec<Rect>>>,
}
impl Source for Pixels {
    fn domain(&self) -> Domain {
        self.domain
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(self.values.clone()))
    }
    fn read(&self, region: Rect) -> Result<Vec<[f32; 4]>, Error> {
        self.reads.lock().unwrap().push(region);
        let mut values = Vec::new();
        for y in region.y..region.bottom() {
            let first = self.domain.finite().unwrap().index([region.x, y]);
            values.extend_from_slice(&self.values[first..first + region.width as usize]);
        }
        Ok(values)
    }
}
fn fixture(size: [u32; 2], values: Vec<[f32; 4]>) -> (Arc<ImageNode>, Arc<Mutex<Vec<Rect>>>) {
    let reads = Arc::new(Mutex::new(Vec::new()));
    (
        source(Arc::new(Pixels {
            domain: Domain {
                bounds: Some(crate::gpu::evaluation::CellRegion::new([0, 0], size)),
                grid: Grid::axis_aligned([0.; 2], [1.; 2]),
                meaning: Meaning::Graded,
            },
            values: Arc::new(values),
            reads: reads.clone(),
        })),
        reads,
    )
}
#[derive(Debug, PartialEq)]
struct Square;
impl Operation for Square {
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(Square))
    }
    fn input(&self, region: Rect) -> Result<Option<Rect>, Error> {
        Ok(Some(region))
    }
    fn pointwise(&self) -> Option<Function> {
        Some(Function {
            linked: Default::default(),
            code:
                "fn square(p: vec4f, position: vec2f) -> vec4f { return vec4f(p.rgb*p.rgb,p.w); }"
                    .into(),
            symbol: "square",
            bindings: vec![],
        })
    }
}

fn output(mapping: Mapping, region: Rect) -> Domain {
    Domain {
        bounds: Some(region),
        grid: Grid {
            origin: mapping.start,
            axes: mapping.axes,
            pitch: mapping.axes.map(|a| a[0].hypot(a[1])),
            coverage: 8,
        },
        meaning: Meaning::Graded,
    }
}
fn materialize(root: Arc<ImageNode>, tile: [u32; 2], scratch_bytes: u64) -> Vec<[f32; 4]> {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.test_materializations(false);
    let domain = root.domain;
    let image = pollster::block_on(
        context
            .materialize(
                &queue,
                &Default::default(),
                &scope,
                context
                    .request(
                        root,
                        Region {
                            bounds: domain.finite().unwrap(),
                            grid: domain.grid,
                        },
                        Budget {
                            tile,
                            scratch_bytes,
                        },
                    )
                    .unwrap(),
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(context.retained_bytes(), 0);
    super::super::super::execute::read(&device, &queue, image.as_buffer().unwrap())
}

#[test]
fn nested_large_aperture_stays_bounded_and_does_not_move_nonlinear_operations_across_means() {
    let size = [1024, 256];
    let (source, reads) = fixture(
        size,
        (0..size[0] * size[1])
            .map(|i| {
                let scale = if i % size[0] < 512 { 1. } else { 2. };
                let value = if i % 2 == 0 { -scale } else { 3. * scale };
                [value, value * 0.5, -value, if i % 2 == 0 { 1. } else { 2. }]
            })
            .collect(),
    );
    let first = Mapping {
        start: [0.; 2],
        axes: [[512., 0.], [0., 256.]],
    };
    let coarse = reconstruct_mapped(source, output(first, [0, 0, 2, 1].into()), first).unwrap();
    assert!(
        coarse
            .operation
            .sample_consumer()
            .unwrap()
            .production()
            .unwrap()
            .requires_worker(),
        "control: the inner aperture must actually require bounded production"
    );
    let squared = ImageNode::new(coarse.domain, Some(coarse), Square);
    let second = Mapping {
        start: [0.; 2],
        axes: [[1024., 0.], [0., 256.]],
    };
    let mean = reconstruct(squared, output(second, [0, 0, 1, 1].into())).unwrap();
    let squared_mean = ImageNode::new(mean.domain, Some(mean.clone()), Square);
    for (root, expected) in [
        (mean, [2.5, 0.625, 2.5, 11.]),
        (squared_mean, [6.25, 0.390625, 6.25, 11.]),
    ] {
        let first_read = reads.lock().unwrap().len();
        let values = materialize(root, [1, 1], 2 << 20);
        let mut covered = vec![false; (size[0] * size[1]) as usize];
        for region in &reads.lock().unwrap()[first_read..] {
            for y in region.y..region.bottom() {
                let first = (y as u32 * size[0] + region.x as u32) as usize;
                covered[first..first + region.width as usize].fill(true);
            }
        }
        assert!(
            covered.into_iter().all(|value| value),
            "each cache-off calculation must acquire every contributing original; dependency halos may overlap"
        );
        for (actual, expected) in values[0].iter().zip(expected) {
            assert!(
                (actual - expected).abs() < 3e-5,
                "nested mean: {:?} expected {expected}",
                values[0]
            );
        }
    }
    let reads = reads.lock().unwrap();
    assert!(
        reads.len() > 2,
        "control: this must not acquire either complete source in one read"
    );
    assert!(reads.iter().all(|r| r.width <= 512 && r.height <= 128));
}

#[test]
fn a_recordable_nested_aperture_uses_worker_production_when_its_input_exceeds_admission() {
    let size = [1024, 512];
    let (input, reads) = fixture(
        size,
        (0..size[0] * size[1])
            .map(|i| {
                let value = if i % 2 == 0 { -1. } else { 3. };
                [value, 2. * value, -value, 0.]
            })
            .collect(),
    );
    let map = Mapping {
        start: [0.; 2],
        axes: [[16., 0.], [0., 16.]],
    };
    let inner = reconstruct_mapped(input, output(map, [0, 0, 64, 32].into()), map).unwrap();
    assert!(
        !inner
            .operation
            .sample_consumer()
            .unwrap()
            .production()
            .unwrap()
            .requires_worker(),
        "control: aperture size alone must not force worker production"
    );
    let root = ImageNode::new(inner.domain, Some(inner), Square);
    let actual = materialize(root, [64, 32], 512 << 10);
    for pixel in actual {
        for (a, b) in pixel.into_iter().zip([1., 4., 1., 8.]) {
            assert!((a - b).abs() < 3e-5, "{pixel:?} expected {b}");
        }
    }
    let reads = reads.lock().unwrap();
    assert!(reads.len() > 1);
    assert!(reads.iter().all(|r| r.width <= 512 && r.height <= 128));
}

#[test]
fn nested_worker_admission_observes_individual_gpu_binding_limits_not_only_total_scratch() {
    let (input, reads) = fixture([512, 512], vec![[0.25, -2., 3., 0.]; 512 * 512]);
    let map = Mapping {
        start: [0.; 2],
        axes: [[16., 0.], [0., 16.]],
    };
    let inner = reconstruct_mapped(input, output(map, [0, 0, 32, 32].into()), map).unwrap();
    let root = ImageNode::new(inner.domain, Some(inner), Square);
    let instance = crate::gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: wgpu::Limits {
            max_storage_buffer_binding_size: 1 << 20,
            ..Default::default()
        },
        ..Default::default()
    }))
    .unwrap();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.test_materializations(false);
    let budget = Budget {
        tile: [32, 32],
        scratch_bytes: 8 << 20,
    };
    assert!(
        512 * 512 * 16 > device.limits().max_storage_buffer_binding_size
            && 512 * 512 * 16 < budget.scratch_bytes,
        "control: the whole upstream input must fit scratch but not a storage binding"
    );
    let request = context
        .request(
            root.clone(),
            Region {
                bounds: root.domain.finite().unwrap(),
                grid: root.domain.grid,
            },
            budget,
        )
        .unwrap();
    let plan = request.plans().next().unwrap().unwrap();
    assert!(matches!(
        plan.tiles[0].fragment.work[0],
        Work::Produce { .. }
    ));
    let image = pollster::block_on(
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
    .unwrap();
    for pixel in super::super::super::execute::read(&device, &queue, image.as_buffer().unwrap()) {
        assert_eq!(pixel, [0.0625, 4., 9., 8.]);
    }
    assert!(reads.lock().unwrap().len() > 1);
    assert_eq!(context.retained_bytes(), 0);
}

#[test]
fn nested_reductions_execute_recursively_without_flattening_their_source_grids() {
    let (input, reads) = fixture(
        [257, 257],
        (0..257 * 257)
            .map(|i| [(i % 257) as f32 / 256., (i / 257) as f32 / 256., 2., 0.])
            .collect(),
    );
    let first = Mapping {
        start: [0.; 2],
        axes: [[257., 0.], [0., 257.]],
    };
    let coarse = reconstruct_mapped(input, output(first, [0, 0, 257, 257].into()), first).unwrap();
    let squared = ImageNode::new(coarse.domain, Some(coarse), Square);
    let second = Mapping {
        start: [0.; 2],
        axes: [[257. * 257., 0.], [0., 257. * 257.]],
    };
    let root = reconstruct(squared, output(second, [0, 0, 1, 1].into())).unwrap();
    assert!(
        root.operation
            .sample_consumer()
            .unwrap()
            .production()
            .unwrap()
            .requires_worker()
    );
    let actual = materialize(root, [1, 1], 32 << 20);
    let expected = 256.25_f32 / 257.;
    for (a, b) in actual[0].into_iter().zip([expected, expected, 4., 8.]) {
        assert!(
            (a - b).abs() < 3e-5,
            "recursive mean: {:?} expected {b}",
            actual[0]
        );
    }
    let reads = reads.lock().unwrap();
    assert!(reads.len() > 3);
    assert!(reads.iter().all(|r| r.width <= 512 && r.height <= 128));
}

#[test]
fn nested_worker_admission_rejects_before_original_reads() {
    let (input, reads) = fixture([512, 256], vec![[1.; 4]; 512 * 256]);
    let map = Mapping {
        start: [0.; 2],
        axes: [[512., 0.], [0., 256.]],
    };
    let inner = reconstruct_mapped(input, output(map, [0, 0, 1, 1].into()), map).unwrap();
    let root = ImageNode::new(inner.domain, Some(inner), Square);
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.test_materializations(false);
    let request_for = |scratch_bytes| {
        context.request(
            root.clone(),
            Region {
                bounds: root.domain.finite().unwrap(),
                grid: root.domain.grid,
            },
            Budget {
                tile: [1, 1],
                scratch_bytes,
            },
        )
    };
    let parent_bytes = request_for(1024)
        .unwrap()
        .plans()
        .next()
        .unwrap()
        .unwrap()
        .peak_scratch_bytes;
    let request = request_for(parent_bytes).unwrap();
    let plan = request.plans().next().unwrap().unwrap();
    assert!(
        plan.peak_scratch_bytes == parent_bytes
            && matches!(plan.tiles[0].fragment.work[0], Work::Produce { .. }),
        "control: the parent plan fits; the child worker's remaining allowance must reject"
    );
    let result = pollster::block_on(
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
    );
    assert!(matches!(result,Err(error) if matches!(error.as_ref(),Error::InvalidParameter(_))));
    assert!(reads.lock().unwrap().is_empty());
    assert_eq!(context.retained_bytes(), 0);
}

#[test]
fn cancellation_inside_a_nested_worker_does_not_publish_parent_pixels_or_read_more_originals() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use wgpu::util::DeviceExt;
    let (input, reads) = fixture([512, 256], vec![[1.; 4]; 512 * 256]);
    let map = Mapping {
        start: [0.; 2],
        axes: [[512., 0.], [0., 256.]],
    };
    let inner = reconstruct_mapped(input, output(map, [0, 0, 1, 1].into()), map).unwrap();
    let root = ImageNode::new(inner.domain, Some(inner), Square);
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.test_materializations(false);
    let pixels = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&[[1234_f32; 4]]),
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
    });
    let stopped = Arc::new(AtomicBool::new(false));
    let submissions = Arc::new(AtomicUsize::new(0));
    let gate = crate::gpu::submission::SubmissionGate::default();
    let (stop, count) = (stopped.clone(), submissions.clone());
    gate.observe_submissions(Arc::new(move || {
        if count.fetch_add(1, Ordering::SeqCst) == 1 {
            stop.store(true, Ordering::SeqCst);
        }
    }));
    let budget = Budget {
        tile: [1, 1],
        scratch_bytes: 2 << 20,
    };
    let evaluation = context
        .request(
            root.clone(),
            Region {
                bounds: root.domain.finite().unwrap(),
                grid: root.domain.grid,
            },
            budget,
        )
        .unwrap()
        .with_materializations(scope.clone());
    let result = Production {
        context: &context,
        queue: &queue,
        gate: &gate,
        scope: &scope,
        budget,
        layout: ImageLayout {
            domain: root.domain,
            region: root.domain.finite().unwrap(),
            format: ImageFormat::Rgba32Float,
            backing: ImageBacking::Buffer,
        },
        pixels: &pixels,
        cancelled: &|| stopped.load(Ordering::SeqCst),
        progress: &|| {},
    }
    .produce(&evaluation);
    assert!(matches!(result, Err(Error::Cancelled)));
    assert_eq!(submissions.load(Ordering::SeqCst), 2);
    assert_eq!(reads.lock().unwrap().len(), 1);
    assert_eq!(
        super::super::super::execute::read(&device, &queue, &pixels),
        vec![[1234.; 4]]
    );
}

#[test]
fn shared_neighbourhood_admission_falls_back_when_individual_requests_fit() {
    let (input, reads) = fixture([128, 64], vec![[-2., 3., 4., 1.]; 128 * 64]);
    let input = super::super::super::operators::gaussian(input, 1.).unwrap();
    let roots: Vec<_> = [128., 127.]
        .into_iter()
        .map(|width| {
            let mapping = Mapping {
                start: [0.; 2],
                axes: [[width, 0.], [0., 64.]],
            };
            reconstruct_mapped(input.clone(), output(mapping, [0, 0, 1, 1].into()), mapping)
                .unwrap()
        })
        .collect();
    let budget = Budget {
        tile: [1, 1],
        scratch_bytes: 33 << 10,
    };
    let expected: Vec<_> = roots
        .iter()
        .map(|root| materialize(root.clone(), budget.tile, budget.scratch_bytes))
        .collect();
    reads.lock().unwrap().clear();
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.test_materializations(false);
    let requests = roots
        .into_iter()
        .map(|root| {
            context
                .request(
                    root.clone(),
                    Region {
                        bounds: root.domain.finite().unwrap(),
                        grid: root.domain.grid,
                    },
                    budget,
                )
                .unwrap()
        })
        .collect();
    let images = pollster::block_on(futures_util::future::try_join_all(
        context
            .materialize_many(&queue, &Default::default(), &scope, requests)
            .unwrap(),
    ))
    .unwrap();
    for (image, expected) in images.iter().zip(expected) {
        let actual =
            super::super::super::execute::read(&device, &queue, image.as_buffer().unwrap());
        assert_eq!(actual.len(), 1);
        for (a, b) in actual[0].iter().zip(expected[0]) {
            assert!(
                (*a - b).abs() < 2e-5,
                "batched={actual:?}, individual={expected:?}"
            );
        }
    }
    assert!(reads.lock().unwrap().len() > 1);
}

#[test]
fn disjoint_reconstruction_targets_skip_source_units_that_cannot_contribute() {
    let (input, reads) = fixture(
        [1536, 16],
        (0..1536 * 16)
            .map(|i| {
                if i % 1536 < 768 {
                    [-2., 3., 4., 1.]
                } else {
                    [5., -4., 8., 2.]
                }
            })
            .collect(),
    );
    let roots = [0., 1520.].into_iter().map(|x| {
        let map = Mapping {
            start: [x, 0.],
            axes: [[16., 0.], [0., 16.]],
        };
        reconstruct_mapped(input.clone(), output(map, [0, 0, 1, 1].into()), map).unwrap()
    });
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.test_materializations(false);
    let requests = roots
        .map(|root| {
            let d = root.domain;
            context
                .request(
                    root,
                    Region {
                        bounds: d.finite().unwrap(),
                        grid: d.grid,
                    },
                    Budget {
                        tile: [1, 1],
                        scratch_bytes: 64 << 10,
                    },
                )
                .unwrap()
        })
        .collect();
    let images = pollster::block_on(futures_util::future::try_join_all(
        context
            .materialize_many(&queue, &Default::default(), &scope, requests)
            .unwrap(),
    ))
    .unwrap();
    for (image, expected) in images.iter().zip([[-2., 3., 4., 9.], [5., -4., 8., 10.]]) {
        let value = super::super::super::execute::read(&device, &queue, image.as_buffer().unwrap());
        assert_eq!(value, vec![expected]);
    }
    let reads = reads.lock().unwrap();
    assert!(reads.len() > 1);
    assert!(reads.iter().map(|r| r.width * r.height).sum::<u32>() < 1536 * 16);
}

#[test]
fn related_reconstructions_share_source_traversal_and_duplicate_roots_share_one_image() {
    let values = (0..8192)
        .map(|i| {
            [
                if i % 2 == 0 { -1. } else { 3. },
                2.,
                -4.,
                if i % 128 == 0 { 1. } else { 2. },
            ]
        })
        .collect();
    let (input, reads) = fixture([128, 64], values);
    let input = ImageNode::new(input.domain, Some(input), Square);
    let detail = Mapping {
        start: [32., 16.],
        axes: [[32., 0.], [0., 16.]],
    };
    let whole = Mapping {
        start: [0.; 2],
        axes: [[128., 0.], [0., 64.]],
    };
    let detail =
        reconstruct_mapped(input.clone(), output(detail, [0, 0, 2, 2].into()), detail).unwrap();
    let whole = reconstruct_mapped(input, output(whole, [0, 0, 1, 1].into()), whole).unwrap();
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.test_materializations(false);
    let evaluations = [detail.clone(), whole, detail]
        .into_iter()
        .map(|root| {
            let d = root.domain;
            context
                .request(
                    root,
                    Region {
                        bounds: d.finite().unwrap(),
                        grid: d.grid,
                    },
                    Budget {
                        tile: [2, 2],
                        scratch_bytes: 64 << 10,
                    },
                )
                .unwrap()
        })
        .collect();
    let futures = context
        .materialize_many(&queue, &Default::default(), &scope, evaluations)
        .unwrap();
    let images = pollster::block_on(futures_util::future::try_join_all(futures)).unwrap();
    assert!(Arc::ptr_eq(&images[0].image, &images[2].image));
    for (index, image) in images.iter().enumerate() {
        let values =
            super::super::super::execute::read(&device, &queue, image.as_buffer().unwrap());
        for p in values {
            for (a, b) in p[..3].iter().zip([5., 4., 16.]) {
                assert!((*a - b).abs() < 2e-5, "{p:?}");
            }
            assert_eq!(p[3], if index == 1 { 11. } else { 10. });
        }
    }
    let reads = reads.lock().unwrap();
    assert!(reads.len() > 1, "exercise bounded shared traversal");
    assert_eq!(
        reads
            .iter()
            .map(|r| u64::from(r.width) * u64::from(r.height))
            .sum::<u64>(),
        8192
    );
    assert_eq!(context.retained_bytes(), 0);
}

#[test]
fn large_aperture_bounds_source_reads_and_applies_upstream_math_before_averaging() {
    let size = [128, 64];
    let values = (0..size[1])
        .flat_map(|y| {
            (0..size[0]).map(move |x| {
                [
                    if (x + y) % 2 == 0 { -1. } else { 3. },
                    2.,
                    -4.,
                    if x == 0 { 1. } else { 2. },
                ]
            })
        })
        .collect();
    let (input, reads) = fixture(size, values);
    let input = ImageNode::new(input.domain, Some(input), Square);
    let map = Mapping {
        start: [0.; 2],
        axes: [[128., 0.], [0., 64.]],
    };
    let root = reconstruct_mapped(input, output(map, [0, 0, 1, 1].into()), map).unwrap();
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let evaluation = context
        .request(
            root.clone(),
            Region {
                bounds: root.domain.finite().unwrap(),
                grid: root.domain.grid,
            },
            Budget {
                tile: [1, 1],
                scratch_bytes: 64 << 10,
            },
        )
        .unwrap();
    assert!(
        evaluation
            .plans()
            .any(|plan| plan.unwrap().tiles.iter().any(|tile| tile
                .fragment
                .work
                .iter()
                .any(|work| matches!(work, Work::Produce { .. })))),
        "this calculation must require bounded worker production"
    );

    for delivery in ["materialize", "read", "sink"] {
        reads.lock().unwrap().clear();
        let actual = match delivery {
            "materialize" => materialize(root.clone(), [1, 1], 64 << 10),
            "read" => {
                let mut actual = Vec::new();
                context
                    .read(
                        &queue,
                        &Default::default(),
                        &evaluation,
                        || false,
                        |region, pixels| {
                            assert_eq!(region, evaluation.region().bounds);
                            actual.extend_from_slice(pixels);
                            Ok(())
                        },
                    )
                    .unwrap()
                    .unwrap();
                actual
            }
            _ => {
                let pixels = device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: 16,
                    usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let deliveries = std::cell::Cell::new(0);
                let submitted = std::cell::Cell::new(0);
                context
                    .evaluate_with(
                        &queue,
                        &Default::default(),
                        &evaluation,
                        || false,
                        Sink {
                            encode: |encoder: &mut wgpu::CommandEncoder,
                                     image: PlannedImage,
                                     input: &wgpu::Buffer| {
                                assert_eq!(image.region, evaluation.region().bounds);
                                encoder.copy_buffer_to_buffer(input, 0, &pixels, 0, 16);
                                deliveries.set(deliveries.get() + 1);
                                Ok(())
                            },
                            submitted: || submitted.set(submitted.get() + 1),
                        },
                    )
                    .unwrap()
                    .unwrap();
                assert_eq!(deliveries.get(), 1);
                assert_eq!(
                    submitted.get(),
                    1,
                    "the caller must be able to arm its readback after submission"
                );
                super::super::super::execute::read(&device, &queue, &pixels)
            }
        };
        assert_eq!(actual.len(), 1);
        for (a, b) in actual[0][..3].iter().zip([5., 4., 16.]) {
            assert!(
                (*a - b).abs() < 2e-5,
                "{delivery}: mean(square) is not square(mean): {actual:?}"
            );
        }
        assert_eq!(actual[0][3], 11.);
        let reads = reads.lock().unwrap();
        assert!(
            reads.len() > 1,
            "{delivery}: the fixture must use streamed source units"
        );
        assert!(
            reads
                .iter()
                .all(|r| u64::from(r.width) * u64::from(r.height) * 16 <= 64 << 10)
        );
        assert_eq!(
            reads
                .iter()
                .map(|r| u64::from(r.width) * u64::from(r.height))
                .sum::<u64>(),
            8192
        );
    }
    reads.lock().unwrap().clear();
    assert!(
        context
            .read(
                &queue,
                &Default::default(),
                &evaluation,
                || !reads.lock().unwrap().is_empty(),
                |_, _| panic!("cancelled production must not deliver a result"),
            )
            .unwrap()
            .is_none()
    );
    assert_eq!(
        reads.lock().unwrap().len(),
        1,
        "cancellation must stop during bounded production"
    );
}

#[test]
fn streamed_diamond_excludes_zero_area_corners_and_bounds_do_not_replace_its_polygon() {
    let size = [128, 128];
    let mut values = vec![[-2., 3., 5., 1.]; 128 * 128];
    for y in 0..8 {
        for x in 0..8 {
            for (x, y) in [(x, y), (127 - x, y), (x, 127 - y), (127 - x, 127 - y)] {
                values[y * 128 + x] = [9999.; 4];
                values[y * 128 + x][3] = 4.;
            }
        }
    }
    values[62 * 128] = [9999., 9999., 9999., 2.]; // touches only one diamond vertex
    let (input, reads) = fixture(size, values);
    let map = Mapping {
        start: [64., 0.],
        axes: [[64., 64.], [-64., 64.]],
    };
    let actual = materialize(
        reconstruct_mapped(input, output(map, [0, 0, 1, 1].into()), map).unwrap(),
        [1, 1],
        64 << 10,
    );
    assert!(reads.lock().unwrap().len() > 1);
    assert_eq!(
        actual[0][3], 9.,
        "only positive-area contributions supply flags"
    );
    for (a, b) in actual[0][..3].iter().zip([-2., 3., 5.]) {
        assert!((*a - b).abs() < 2e-5, "{actual:?}");
    }
}

#[test]
fn streamed_source_units_keep_real_neighbours_for_an_upstream_spatial_operation() {
    let mut values = vec![[0., -2., 4., 1.]; 128 * 64];
    values[30 * 128 + 64][0] = 8192.;
    let (input, reads) = fixture([128, 64], values);
    let input = super::super::super::operators::gaussian(input, 1.).unwrap();
    let map = Mapping {
        start: [0.; 2],
        axes: [[128., 0.], [0., 64.]],
    };
    let actual = materialize(
        reconstruct_mapped(input, output(map, [0, 0, 1, 1].into()), map).unwrap(),
        [1, 1],
        32 << 10,
    );
    // The isolated interior impulse keeps its mass under a unit-sum blur, across source units.
    for (a, b) in actual[0][..3].iter().zip([1., -2., 4.]) {
        assert!((*a - b).abs() < 2e-5, "{actual:?}");
    }
    assert_eq!(actual[0][3], 9.);
    let reads = reads.lock().unwrap();
    assert!(reads.len() > 1);
    assert!(reads.iter().all(|r| r.width * r.height * 16 < 32 << 10));
}

#[test]
fn mean_differences_can_overflow_without_overflowing_the_representable_mean() {
    let (input, _) = fixture(
        [2, 1],
        vec![
            [f32::MAX, f32::MAX, 1e-20, 1.],
            [-f32::MAX, f32::MAX, 1e-20, 2.],
        ],
    );
    let map = Mapping {
        start: [0.; 2],
        axes: [[2., 0.], [0., 1.]],
    };
    let actual = materialize(
        reconstruct_mapped(input, output(map, [0, 0, 1, 1].into()), map).unwrap(),
        [1, 1],
        64 << 10,
    );
    assert_eq!(actual, vec![[0., f32::MAX, 1e-20, 11.]]);
}

#[test]
fn streamed_reconstruction_keeps_signed_hdr_means_and_failure_markers() {
    let size = [128, 64];
    let values: Vec<_> = (0..8192)
        .map(|i| {
            [
                if i % 2 == 0 { 2e36 } else { -1e36 },
                -3e35,
                4e36,
                if i == 300 {
                    -2.
                } else if i == 400 {
                    -7.
                } else {
                    2.
                },
            ]
        })
        .collect();
    let (input, _) = fixture(size, values);
    let map = Mapping {
        start: [0.; 2],
        axes: [[128., 0.], [0., 64.]],
    };
    let actual = materialize(
        reconstruct_mapped(input, output(map, [0, 0, 1, 1].into()), map).unwrap(),
        [1, 1],
        64 << 10,
    );
    for (a, b) in actual[0][..3].iter().zip([5e35_f32, -3e35, 4e36]) {
        assert!(a.is_finite() && ((*a - b) / b).abs() < 2e-5, "{actual:?}");
    }
    assert_eq!(actual[0][3], -7.);
}

#[test]
fn a_large_resident_aperture_uses_bounded_contributions_without_reacquiring_source_pixels() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (input, reads) = fixture(
        [512, 256],
        (0..512 * 256)
            .map(|i| [if i % 2 == 0 { -1. } else { 3. }, -2., 4., 1.])
            .collect(),
    );
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.test_materializations(false);
    let budget = Budget {
        tile: [512, 256],
        scratch_bytes: 64 << 20,
    };
    let gate = crate::gpu::submission::SubmissionGate::default();
    let resident = pollster::block_on(
        context
            .materialize(
                &queue,
                &gate,
                &scope,
                context
                    .request(
                        input.clone(),
                        Region {
                            bounds: input.domain.finite().unwrap(),
                            grid: input.domain.grid,
                        },
                        budget,
                    )
                    .unwrap(),
                ImageFormat::Rgba32Float,
                ImageBacking::Texture,
            )
            .unwrap(),
    )
    .unwrap();
    let before = reads.lock().unwrap().len();
    let submissions = Arc::new(AtomicUsize::new(0));
    let observed = submissions.clone();
    gate.observe_submissions(Arc::new(move || {
        observed.fetch_add(1, Ordering::SeqCst);
    }));
    let map = Mapping {
        start: [0.; 2],
        axes: [[512., 0.], [0., 256.]],
    };
    let root = reconstruct_mapped(input, output(map, [0, 0, 1, 1].into()), map).unwrap();
    let direct = context.record(
        context
            .request(
                root.clone(),
                Region {
                    bounds: root.domain.finite().unwrap(),
                    grid: root.domain.grid,
                },
                budget,
            )
            .unwrap(),
        scope.clone(),
        Vec::new(),
    );
    assert!(
        matches!(direct, Err(Error::InvalidParameter(_))),
        "record-only execution must not silently lower an unbounded sampled loop"
    );
    let image = pollster::block_on(
        context
            .materialize(
                &queue,
                &gate,
                &scope,
                context
                    .request(
                        root.clone(),
                        Region {
                            bounds: root.domain.finite().unwrap(),
                            grid: root.domain.grid,
                        },
                        budget,
                    )
                    .unwrap(),
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(reads.lock().unwrap().len(), before);
    assert!(
        submissions.load(Ordering::SeqCst) > 1,
        "control: exercise reduction despite fitting resident input"
    );
    let actual = super::super::super::execute::read(&device, &queue, image.as_buffer().unwrap());
    for (a, b) in actual[0][..3].iter().zip([1., -2., 4.]) {
        assert!((*a - b).abs() < 2e-5, "{actual:?}");
    }
    assert_eq!(actual[0][3], 9.);
    assert_eq!(context.retained_bytes(), 0);
    drop(resident);
}

#[test]
fn irreducible_neighbourhoods_reject_before_source_io_on_the_authored_graph() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Unreadable(Arc<AtomicUsize>);
    impl Source for Unreadable {
        fn domain(&self) -> Domain {
            Domain {
                bounds: Some(Rect::new([0; 2], [100_000; 2])),
                grid: Grid::axis_aligned([0.; 2], [1.; 2]),
                meaning: Meaning::Graded,
            }
        }
        fn read(&self, _: Rect) -> Result<Vec<[f32; 4]>, Error> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Err(Error::Gpu("unexpected source IO".into()))
        }
    }

    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let reads = Arc::new(AtomicUsize::new(0));
    for (sigma, expected) in [
        (100., "reconstruction neighbourhood exceeds working storage"),
        (20_000., "evaluation raster exceeds GPU cell addressing"),
    ] {
        let input = super::super::super::operators::gaussian(
            source(Arc::new(Unreadable(reads.clone()))),
            sigma,
        )
        .unwrap();
        let domain = Domain {
            bounds: Some(Rect::new([50_000; 2], [16; 2])),
            ..input.domain
        };
        let root = reconstruct_complete(input, domain).unwrap();
        let evaluation = context
            .request(
                root,
                Region {
                    bounds: domain.finite().unwrap(),
                    grid: domain.grid,
                },
                Budget {
                    tile: [16; 2],
                    scratch_bytes: 64 << 10,
                },
            )
            .unwrap();
        let result = context.read(
            &queue,
            &Default::default(),
            &evaluation,
            || false,
            |_, _| panic!("inadmissible neighbourhood delivered"),
        );
        assert!(matches!(result, Err(Error::InvalidParameter(message)) if message == expected));
        assert_eq!(reads.load(Ordering::Relaxed), 0);
    }
}

#[test]
fn insufficient_reduction_storage_rejects_before_acquiring_source_pixels() {
    let (input, reads) = fixture([128, 64], vec![[1.; 4]; 128 * 64]);
    let map = Mapping {
        start: [0.; 2],
        axes: [[128., 0.], [0., 64.]],
    };
    let root = reconstruct_mapped(input, output(map, [0, 0, 1, 1].into()), map).unwrap();
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let domain = root.domain;
    let result = pollster::block_on(
        context
            .materialize(
                &queue,
                &Default::default(),
                &context.test_materializations(false),
                context
                    .request(
                        root,
                        Region {
                            bounds: domain.finite().unwrap(),
                            grid: domain.grid,
                        },
                        Budget {
                            tile: [1, 1],
                            scratch_bytes: 16,
                        },
                    )
                    .unwrap(),
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    );
    assert!(matches!(result,Err(error) if matches!(error.as_ref(),Error::InvalidParameter(_))));
    assert!(reads.lock().unwrap().is_empty());
}

#[test]
fn cancellation_after_a_contribution_prevents_final_output_writes_and_more_source_reads() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use wgpu::util::DeviceExt;
    let (input, reads) = fixture([128, 64], vec![[1.; 4]; 128 * 64]);
    let map = Mapping {
        start: [0.; 2],
        axes: [[128., 0.], [0., 64.]],
    };
    let root = reconstruct_mapped(input, output(map, [0, 0, 1, 1].into()), map).unwrap();
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let pixels = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&[[1234_f32; 4]]),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    });
    let stopped = Arc::new(AtomicBool::new(false));
    let submissions = Arc::new(AtomicUsize::new(0));
    let gate = crate::gpu::submission::SubmissionGate::default();
    let (stop, count) = (stopped.clone(), submissions.clone());
    gate.observe_submissions(Arc::new(move || {
        if count.fetch_add(1, Ordering::SeqCst) == 1 {
            stop.store(true, Ordering::SeqCst);
        }
    }));
    let scope = context.test_materializations(false);
    let cancelled = || stopped.load(Ordering::SeqCst);
    let worker = Production {
        context: &context,
        queue: &queue,
        gate: &gate,
        scope: &scope,
        budget: Budget {
            tile: [1, 1],
            scratch_bytes: 64 << 10,
        },
        layout: ImageLayout {
            domain: root.domain,
            region: root.domain.finite().unwrap(),
            format: ImageFormat::Rgba32Float,
            backing: ImageBacking::Buffer,
        },
        pixels: &pixels,
        cancelled: &cancelled,
        progress: &|| {},
    };
    let result = root
        .operation
        .sample_consumer()
        .unwrap()
        .production()
        .unwrap()
        .produce(&worker, root.input.as_ref().unwrap());
    assert!(matches!(result, Err(Error::Cancelled)));
    assert_eq!(submissions.load(Ordering::SeqCst), 2);
    assert_eq!(reads.lock().unwrap().len(), 1);
    assert_eq!(
        super::super::super::execute::read(&device, &queue, &pixels),
        vec![[1234.; 4]]
    );
}

#[test]
fn reflected_fractional_streaming_extends_real_edges_and_addresses_nonzero_tiled_outputs() {
    let size = [128, 65];
    let (input, reads) = fixture(
        size,
        (0..65)
            .flat_map(|y| {
                (0..128).map(move |x| [x as f32 - 64., y as f32 - 32., (x * y) as f32 / 128., 1.])
            })
            .collect(),
    );
    let region: Rect = [7, 9, 3, 2].into();
    let map = Mapping {
        start: [132.4 + 7. * 45.2, -3.25 - 9. * 40.3],
        axes: [[-45.2, 0.], [0., 40.3]],
    };
    let root = reconstruct_mapped(input, output(map, region), map).unwrap();
    let whole = materialize(root.clone(), [3, 2], 32 << 10);
    let tiles = materialize(root, [1, 1], 32 << 10);
    assert!(reads.lock().unwrap().len() > 2);
    // Integral of the unit-cell staircase, including constant extension outside the source.
    let integral = |t: f64, n: f64| {
        let u = t.clamp(0., n);
        let k = u.floor();
        k * (k - 1.) / 2. + k * (u - k) + (n - 1.) * (t - n).max(0.)
    };
    let mean =
        |a: f64, b: f64, n: f64| (integral(a.max(b), n) - integral(a.min(b), n)) / (a - b).abs();
    for image in [&whole, &tiles] {
        for (i, p) in image.iter().enumerate() {
            let x = i % 3;
            let y = i / 3;
            let mx = mean(132.4 - x as f64 * 45.2, 132.4 - (x + 1) as f64 * 45.2, 128.);
            let my = mean(-3.25 + y as f64 * 40.3, -3.25 + (y + 1) as f64 * 40.3, 65.);
            assert_eq!(p[3], 9.);
            for (a, b) in p[..3].iter().zip([mx - 64., my - 32., mx * my / 128.]) {
                assert!(
                    (f64::from(*a) - b).abs() < 3e-4,
                    "pixel={i} actual={p:?} expected={b}"
                );
            }
        }
    }
}
