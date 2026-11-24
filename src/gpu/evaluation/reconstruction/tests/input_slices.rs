use super::*;
use std::sync::Mutex;

struct Plane {
    reads: Arc<Mutex<Vec<Rect>>>,
}
impl Source for Plane {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some([17, 23, 512, 384].into()),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        }
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("fractional-pan source plane"))
    }
    fn read(&self, region: Rect) -> Result<Vec<[f32; 4]>, Error> {
        self.reads.lock().unwrap().push(region);
        Ok((region.y..region.bottom())
            .flat_map(|y| {
                (region.x..region.right())
                    .map(move |x| [x as f32 / 64. - 4., y as f32 / 32. - 3., -2., 1.])
            })
            .collect())
    }
}

fn squared(source: Value) -> Value {
    value::pointwise(
        source,
        value::Operation::new(
            "squared source plane",
            (),
            "fn square(cell:vec2i, position:vec2f, p:vec4f)->vec4f { return vec4f(p.rgb*p.rgb,p.w); }",
            "square",
            value::Access::Pointwise,
        ),
    )
    .unwrap()
}

fn view(input: Value, origin: [f64; 2]) -> Value {
    reconstruct_tiled(
        input,
        Domain {
            bounds: None,
            grid: Grid {
                coverage: 8,
                ..Grid::axis_aligned(origin, [0.5; 2])
            },
        },
    )
    .unwrap()
}

fn render(
    context: &Context,
    queue: &crate::gpu::submission::Queue,
    input: Value,
    origin: [f64; 2],
) -> Image<ImageLayout> {
    render_root(context, queue, view(input, origin), [0, 0, 64, 32].into())
}

fn render_root(
    context: &Context,
    queue: &crate::gpu::submission::Queue,
    root: Value,
    bounds: Rect,
) -> Image<ImageLayout> {
    let work = context
        .prepare_image(
            context
                .request(
                    root.clone(),
                    Region {
                        bounds,
                        grid: root.domain().grid,
                    },
                    Tiling {
                        tile: [bounds.width, bounds.height],
                    },
                )
                .unwrap(),
        )
        .unwrap();
    context.produce_prepared(queue, work, || false).unwrap()
}

fn check(
    device: &wgpu::Device,
    queue: &crate::gpu::submission::Queue,
    image: &Image<ImageLayout>,
    origin: [f64; 2],
) {
    // Integrate the squared unit-cell staircase, not the square of its reconstructed mean.
    let mean = |low: f64, divisor: f64, offset: f64| {
        let high = low + 0.5;
        (low.floor() as i32..high.ceil() as i32)
            .map(|cell| {
                let value = f64::from(cell) / divisor - offset;
                let weight = (high.min(f64::from(cell + 1)) - low.max(f64::from(cell))).max(0.);
                value * value * weight / 0.5
            })
            .sum::<f64>()
    };
    let pixels = super::super::super::execute::read(device, queue, image.as_buffer().unwrap());
    assert_eq!(pixels.len(), 64 * 32);
    for (i, pixel) in pixels.iter().enumerate() {
        let expected = [
            mean(origin[0] + (i % 64) as f64 * 0.5, 64., 4.),
            mean(origin[1] + (i / 64) as f64 * 0.5, 32., 3.),
            4.,
            9.,
        ];
        for (actual, expected) in pixel.iter().zip(expected) {
            assert!(
                (f64::from(*actual) - expected).abs() < 2e-6,
                "pixel {i}: {pixel:?}"
            );
        }
    }
}

#[test]
fn fractional_pans_reuse_source_pixels_without_materializing_a_cheap_pointwise_prefix() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 8 << 20,
        },
    );
    let scope = context.materializations();
    scope.set_test_pressure(crate::sys::MemoryPressure::Normal);
    let reads = Arc::new(Mutex::new(Vec::new()));
    let source = source(Arc::new(Plane {
        reads: reads.clone(),
    }));
    let input = squared(source.clone());
    let first = render(&context, &queue, input.clone(), [180.25, 125.25]);
    check(&device, &queue, &first, [180.25, 125.25]);
    let acquired = reads.lock().unwrap().clone();
    assert!(
        !acquired.is_empty(),
        "the cold view must acquire source pixels"
    );
    let stored = scope.ready::<ImageLayout>(source.identity().unwrap());
    let original = stored
        .iter()
        .find(|image| {
            acquired
                .iter()
                .all(|region| image.descriptor.region.contains_region(*region))
        })
        .expect(
            "the acquired source slice must be discoverable independently of upload partitions",
        );
    let original = Arc::downgrade(&original.image);
    drop(stored);

    let panned = render(&context, &queue, input.clone(), [181.375, 125.625]);
    check(&device, &queue, &panned, [181.375, 125.625]);
    assert_ne!(
        first.image, panned.image,
        "the new sampling phase must actually execute"
    );
    assert_eq!(
        *reads.lock().unwrap(),
        acquired,
        "covered pans must not reacquire source pixels"
    );
    let ready = scope.ready::<ImageLayout>(source.identity().unwrap());
    assert!(
        ready
            .iter()
            .any(|image| original.as_ptr() == Arc::as_ptr(&image.image))
    );
    assert!(
        scope
            .ready::<ImageLayout>(input.identity().unwrap())
            .is_empty(),
        "ordinary fusion can keep the inexpensive prefix unstored"
    );
    drop(ready);
    drop(first);

    let exposed = render(&context, &queue, input.clone(), [300.125, 126.375]);
    check(&device, &queue, &exposed, [300.125, 126.375]);
    assert!(
        reads.lock().unwrap().len() > acquired.len(),
        "new source coverage must execute"
    );
    let count = reads.lock().unwrap().len();
    let returned = render(&context, &queue, input, [180.375, 125.625]);
    check(&device, &queue, &returned, [180.375, 125.625]);
    assert_eq!(
        reads.lock().unwrap().len(),
        count,
        "retained source slices survive a return at a different fractional phase"
    );
}

#[test]
fn fractional_pans_keep_their_exact_apertures_with_zero_optional_retention() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let reads = Arc::new(Mutex::new(Vec::new()));
    let source = source(Arc::new(Plane {
        reads: reads.clone(),
    }));
    let input = squared(source.clone());
    let mut previous = None;
    let mut initial_reads = None;
    for (step, origin) in [[180.25, 125.25], [181.375, 125.625], [300.125, 126.375]]
        .into_iter()
        .enumerate()
    {
        let next = render(&context, &queue, input.clone(), origin);
        check(&device, &queue, &next, origin);
        assert_eq!(context.retained_bytes(), 0);
        let count = reads.lock().unwrap().len();
        match step {
            0 => initial_reads = Some(count),
            2 => assert!(
                count > initial_reads.unwrap(),
                "new coverage still requires acquisition"
            ),
            _ => {}
        }
        previous = Some(next);
    }
    assert!(!reads.lock().unwrap().is_empty());
    drop(previous);
    assert!(
        context
            .materializations()
            .ready::<ImageLayout>(source.identity().unwrap())
            .is_empty(),
        "source leases retire with their last live frame"
    );
}

struct Diamond {
    reads: Arc<Mutex<Vec<Rect>>>,
    ramp: bool,
}
impl Source for Diamond {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some([0, 0, 256, 256].into()),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        }
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new((
            "diamond with marked exterior",
            self.ramp,
        )))
    }
    fn read(&self, region: Rect) -> Result<Vec<[f32; 4]>, Error> {
        self.reads.lock().unwrap().push(region);
        Ok((region.y..region.bottom())
            .flat_map(|y| {
                (region.x..region.right()).map(move |x| {
                    if x < 100 && y < 85 {
                        [9999., 9999., 9999., 4.]
                    } else if self.ramp {
                        let x = (x as f32 - 128.) / 128.;
                        let y = (y as f32 - 128.) / 128.;
                        [x, y, x * y, 1.]
                    } else {
                        [-2., 3., 5., 1.]
                    }
                })
            })
            .collect())
    }
}

#[test]
fn rotated_slice_padding_does_not_contribute_pixels_or_flags() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let reads = Arc::new(Mutex::new(Vec::new()));
    for ramp in [false, true] {
        let source = source(Arc::new(Diamond {
            reads: reads.clone(),
            ramp,
        }));
        // The diamond's left edge is x=192-y above its midline. The marked
        // rectangle x<100,y<85 is strictly outside it, but inside fetched slices.
        let root = reconstruct_tiled(
            source,
            Domain {
                bounds: None,
                grid: Grid {
                    origin: [128., 64.],
                    axes: [[64., 64.], [-64., 64.]],
                    pitch: [64. * 2f64.sqrt(); 2],
                    coverage: 8,
                },
            },
        )
        .unwrap();
        let image = render_root(&context, &queue, root, [0, 0, 1, 1].into());
        let pixels =
            super::super::super::execute::read(&device, &queue, image.as_buffer().unwrap());
        assert_eq!(pixels.len(), 1);
        let expected = if ramp {
            // Reflection about x=y=128 pairs source cells i and 255-i. Their
            // means are -0.5/128; the centred cross moment vanishes by symmetry.
            [-1. / 256., -1. / 256., 1. / 65536., 9.]
        } else {
            [-2., 3., 5., 9.]
        };
        for (actual, expected) in pixels[0].iter().zip(expected) {
            assert!((actual - expected).abs() < 2e-6, "{actual} vs {expected}");
        }
        assert_eq!(pixels[0][3], 9.);
    }
    let reads = reads.lock().unwrap();
    assert!(
        reads
            .iter()
            .any(|region| region.contains_region([90, 75, 1, 1].into())),
        "the fixture must actually fetch the marked padding"
    );
    assert!(
        reads
            .iter()
            .map(|region| u64::from(region.width) * u64::from(region.height))
            .sum::<u64>()
            <= 2 * 256 * 256,
        "canonical slices must not repeatedly expand overlapping row spans"
    );
    assert_eq!(context.retained_bytes(), 0);
}

struct WidePlane(Arc<Mutex<Vec<Rect>>>);
impl Source for WidePlane {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some([0, 0, 4096, 3072].into()),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        }
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("viewport-wide source coverage"))
    }
    fn acquisition_tile(&self) -> Option<[u32; 2]> {
        Some([256, 256])
    }
    fn read(&self, region: Rect) -> Result<Vec<[f32; 4]>, Error> {
        self.0.lock().unwrap().push(region);
        Ok(vec![
            [1., -2., 3., 1.];
            region.width as usize * region.height as usize
        ])
    }
}

#[test]
fn source_prefetch_preserves_renderability_without_reserving_a_consumer_byte_allowance() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let bounds: Rect = [0, 0, 1280, 720].into();
    for prepared in [false, true] {
        let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
        let source = source(Arc::new(WidePlane(Arc::new(Mutex::new(Vec::new())))));
        let root = view(source, [1536.25, 1152.375]);
        let image = if prepared {
            render_root(&context, &queue, root, bounds)
        } else {
            let request = context
                .request(
                    root.clone(),
                    Region {
                        bounds,
                        grid: root.domain().grid,
                    },
                    Tiling {
                        tile: [bounds.width, bounds.height],
                    },
                )
                .unwrap();
            pollster::block_on(
                context
                    .materialize(
                        &queue,
                        &context.materializations(),
                        request,
                        ImageFormat::Rgba32Float,
                        ImageBacking::Buffer,
                    )
                    .unwrap(),
            )
            .expect("ordinary bounded execution must admit the control")
        };
        let pixels =
            super::super::super::execute::read(&device, &queue, image.as_buffer().unwrap());
        assert!(pixels.iter().all(|pixel| *pixel == [1., -2., 3., 9.]));
    }
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
}

#[test]
fn a_large_view_acquires_source_slices_once_and_reuses_ordinary_retention_across_pans() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 64 << 20,
        },
    );
    let scope = context.materializations();
    scope.set_test_pressure(crate::sys::MemoryPressure::Normal);
    let reads = Arc::new(Mutex::new(Vec::new()));
    let source = source(Arc::new(WidePlane(reads.clone())));
    let bounds: Rect = [0, 0, 1280, 720].into();
    let origin = [1536.25, 1152.375];
    let input = squared(source.clone());
    let root = view(input.clone(), origin);
    let declared = root.operation().reads(bounds).unwrap();
    let first = render_root(&context, &queue, root, bounds);
    let acquired = reads.lock().unwrap().clone();
    assert!(!acquired.is_empty());
    for (i, a) in acquired.iter().enumerate() {
        assert!(
            acquired[i + 1..].iter().all(|b| !overlaps(*a, *b)),
            "output units must not reacquire overlapping source coverage: {acquired:?}"
        );
    }
    let fetched: u64 = acquired
        .iter()
        .map(|r| u64::from(r.width) * u64::from(r.height))
        .sum();
    let declared: u64 = declared
        .iter()
        .map(|r| u64::from(r.region.width) * u64::from(r.region.height))
        .sum();
    assert_eq!(
        fetched, declared,
        "backend subdivision must acquire the declared source coverage exactly once"
    );
    let mut previous = first;
    for origin in [[1537.125, 1152.625], [1567.875, 1152.875]] {
        let next = render_root(&context, &queue, view(input.clone(), origin), bounds);
        assert_ne!(previous.image, next.image);
        assert_eq!(
            *reads.lock().unwrap(),
            acquired,
            "covered fractional pans must use ordinarily retained source slices"
        );
        previous = next;
    }
    let exposed = render_root(
        &context,
        &queue,
        view(input.clone(), [1700.125, 1152.875]),
        bounds,
    );
    let after = reads.lock().unwrap().clone();
    assert!(after.len() > acquired.len());
    assert!(
        after[acquired.len()..]
            .iter()
            .all(|new| acquired.iter().all(|old| !overlaps(*new, *old))),
        "crossing a source-slice edge must acquire new slices, not reread the viewport interior"
    );
    let pixels = super::super::super::execute::read(&device, &queue, exposed.as_buffer().unwrap());
    assert!(pixels.iter().all(|pixel| *pixel == [1., 4., 9., 9.]));
    assert!(
        context
            .materializations()
            .ready::<ImageLayout>(input.identity().unwrap())
            .is_empty(),
        "viewport-wide source acquisition must preserve pointwise fusion"
    );
    let sources = scope
        .ready::<ImageLayout>(source.identity().unwrap())
        .iter()
        .map(|image| Arc::downgrade(&image.image))
        .collect::<Vec<_>>();
    assert!(!sources.is_empty());
    context.set_retention_limit(0);
    assert!(
        sources.iter().all(|image| image.upgrade().is_none()),
        "completed frames must not pin prefetched source slices after ordinary eviction"
    );
    let reacquired = render_root(&context, &queue, view(input, [1700.25, 1152.875]), bounds);
    assert!(
        reads.lock().unwrap().len() > after.len(),
        "the next sampling phase must reacquire evicted source slices"
    );
    assert!(
        scope
            .ready::<ImageLayout>(source.identity().unwrap())
            .is_empty(),
        "zero optional retention must not keep completed source acquisitions alive"
    );
    drop((previous, exposed, reacquired));
    assert_eq!(context.retained_bytes(), 0);
}

#[test]
fn source_slices_survive_execution_but_retire_while_completed_outputs_remain_live() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let (device, queue, _) = crate::gpu::test_support::render_context();
    for downstream in [false, true] {
        let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
        let source = source(Arc::new(WidePlane(Arc::new(Mutex::new(Vec::new())))));
        let root = view(squared(source.clone()), [1536.25, 1152.375]);
        let root = if downstream { squared(root) } else { root };
        let scope = context.materializations();
        let during_execution = Arc::new(AtomicBool::new(false));
        let seen = during_execution.clone();
        let active = scope.clone();
        let identity = source.identity().unwrap().clone();
        let gate = queue.clone();
        gate.observe_submissions(Arc::new(move || {
            if !active.ready::<ImageLayout>(&identity).is_empty() {
                seen.store(true, Ordering::Relaxed);
            }
        }));
        let bounds: Rect = [0, 0, 1280, 720].into();
        let request = context
            .request(
                root.clone(),
                Region {
                    bounds,
                    grid: root.domain().grid,
                },
                Tiling { tile: [1280, 720] },
            )
            .unwrap();
        let work = context.prepare_image(request).unwrap();
        let image = context.produce_prepared(&queue, work, || false).unwrap();
        assert!(during_execution.load(Ordering::Relaxed));
        assert!(
            scope
                .ready::<ImageLayout>(source.identity().unwrap())
                .is_empty(),
            "the independently completed output must not extend source-slice residency"
        );
        let pixels =
            super::super::super::execute::read(&device, &queue, image.as_buffer().unwrap());
        let expected = if downstream {
            [1., 16., 81., 9.]
        } else {
            [1., 4., 9., 9.]
        };
        assert!(pixels.iter().all(|pixel| *pixel == expected));
        assert_eq!(context.retained_bytes(), 0);
    }
}
