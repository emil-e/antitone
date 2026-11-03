use super::*;
use wgpu::util::DeviceExt;

fn buffer(device: &wgpu::Device, value: f32) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&[[value, -0.25, 3., 9.]; 4]),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    })
}

fn produce(scope: &Materializations, key: u32, pixels: wgpu::Buffer) -> Image<u32> {
    pollster::block_on(scope.obtain(
        RetainedKey::new(key),
        Arc::new(4u32),
        pixels.size(),
        move || async move { Ok(GpuImage::new(pixels)) },
    ))
    .unwrap()
}

#[test]
fn recorded_image_publication_starts_after_submission_and_tracks_independent_live_owners() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Pixels {
        domain: Domain,
        reads: Arc<AtomicUsize>,
    }
    impl Pixels {
        fn values(region: CellRegion) -> Vec<[f32; 4]> {
            (region.y..region.bottom())
                .flat_map(|y| {
                    (region.x..region.right())
                        .map(move |x| [0.25 + x as f32, -0.5 - y as f32, 3., 8.])
                })
                .collect()
        }
    }
    impl Source for Pixels {
        fn domain(&self) -> Domain {
            self.domain
        }
        fn identity(&self) -> Option<RetainedKey> {
            Some(RetainedKey::new("coordinate-valued publication fixture"))
        }
        fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
            assert!(self.domain.contains_region(region));
            self.reads.fetch_add(1, Ordering::Relaxed);
            Ok(Self::values(region))
        }
    }
    #[derive(Debug, PartialEq)]
    struct Twice;
    impl Operation for Twice {
        fn identity(&self) -> Option<RetainedKey> {
            Some(RetainedKey::new(Self))
        }
        fn input(&self, region: CellRegion) -> Result<Option<CellRegion>, Error> {
            Ok(Some(region))
        }
        fn pointwise(&self) -> Option<Function> {
            Some(Function::new(
                "fn twice(p: vec4f, xy: vec2f) -> vec4f { return vec4f(p.rgb * 2., p.w); }",
                "twice",
            ))
        }
    }
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.materializations();
    let region = CellRegion::new([0, 0], [2, 2]);
    let domain = Domain {
        bounds: Some(region),
        grid: Grid {
            coverage: 8,
            ..Grid::axis_aligned([0.; 2], [1.; 2])
        },
        meaning: Meaning::Graded,
    };
    let reads = Arc::new(AtomicUsize::new(0));
    let source = super::super::source(Arc::new(Pixels {
        domain,
        reads: reads.clone(),
    }));
    let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&Pixels::values(region)),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let argument = SuppliedImage::from_buffer(source.clone(), region, input.into()).unwrap();
    let root = ImageNode::new(domain, Some(source), Twice);
    let request = || {
        context
            .request(
                root.clone(),
                Region {
                    bounds: region,
                    grid: domain.grid,
                },
                Budget {
                    tile: [2, 2],
                    scratch_bytes: 1 << 20,
                },
            )
            .unwrap()
    };
    let recorded = context
        .record(request(), scope.clone(), vec![argument])
        .unwrap();
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 64,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let output = BufferImage::recorded(&root, region, output).unwrap();
    let identity = root.identity().unwrap();
    assert!(matches!(output, BufferImage::Produced(_)));
    assert!(
        scope
            .find_ready::<ImageLayout>(identity, |_| true)
            .is_none(),
        "unsubmitted work must not make other consumers wait for a paint"
    );
    let mut encoder = device.create_command_encoder(&Default::default());
    recorded
        .encode_result(&device, &mut encoder, &output)
        .unwrap();
    let gate = crate::gpu::SubmissionGate::default();
    context
        .submit_with(
            &queue,
            &gate,
            encoder,
            || false,
            || {
                recorded.submitted();
                output.submitted(&scope);
            },
        )
        .unwrap()
        .unwrap();
    let independent = scope.find_ready::<ImageLayout>(identity, |_| true).unwrap();
    drop(recorded);
    drop(output);
    assert!(
        scope
            .find_ready::<ImageLayout>(identity, |_| true)
            .is_some()
    );
    let requested = pollster::block_on(
        context
            .materialize(
                &queue,
                &gate,
                &scope,
                request(),
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        reads.load(Ordering::Relaxed),
        0,
        "the stored calculation must avoid source production"
    );
    let expected = [
        [0.5, -1., 6., 8.],
        [2.5, -1., 6., 8.],
        [0.5, -3., 6., 8.],
        [2.5, -3., 6., 8.],
    ];
    assert_eq!(
        execute::read(&device, &queue, requested.as_buffer().unwrap()),
        expected
    );
    assert_eq!(context.retained_bytes(), 0);
    drop(independent);
    assert!(
        scope
            .find_ready::<ImageLayout>(identity, |_| true)
            .is_some(),
        "one consumer cannot remove another consumer's value"
    );
    drop(requested);
    assert!(
        scope
            .find_ready::<ImageLayout>(identity, |_| true)
            .is_none()
    );
    let rebuilt = pollster::block_on(
        context
            .materialize(
                &queue,
                &gate,
                &scope,
                request(),
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(reads.load(Ordering::Relaxed), 1);
    assert_eq!(
        execute::read(&device, &queue, rebuilt.as_buffer().unwrap()),
        expected
    );
}

#[test]
fn common_image_store_uses_byte_budgeted_recency_without_owning_active_consumers() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 128,
        },
    );
    let cache = context.materializations();
    let a = produce(&cache, 1, buffer(&device, 1.));
    drop(produce(&cache, 2, buffer(&device, 2.)));
    assert_eq!(context.retained_bytes(), 128);
    assert!(
        cache
            .find::<u32>(&RetainedKey::new(1u32), |support| *support >= 3)
            .is_some()
    );
    assert!(
        cache
            .find::<u32>(&RetainedKey::new(1u32), |support| *support >= 5)
            .is_none()
    );
    assert!(
        cache
            .find::<u64>(&RetainedKey::new(1u32), |_| true)
            .is_none()
    );
    drop(produce(&cache, 3, buffer(&device, 3.)));
    assert!(
        cache
            .find::<u32>(&RetainedKey::new(2u32), |_| true)
            .is_none()
    );
    assert_eq!(context.retained_bytes(), 128);
    cache.clear();
    assert_eq!(context.retained_bytes(), 0);
    assert_eq!(
        execute::read(&device, &queue, a.as_buffer().unwrap()),
        [[1., -0.25, 3., 9.]; 4]
    );
    assert!(cache.is_empty());
}

#[test]
fn common_store_keeps_fitting_values_after_request_handles_leave() {
    let (device, _, _) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 40 * 64,
        },
    );
    for key in 0..40 {
        let caller = context.clone().materializations();
        drop(produce(&caller, key, buffer(&device, key as f32)));
    }
    assert_eq!(context.retained_bytes(), 40 * 64);
    let later = context.materializations();
    for key in 0u32..40 {
        assert!(
            later
                .find::<u32>(&RetainedKey::new(key), |_| true)
                .is_some()
        );
    }
    later.set_retention_enabled(false);
    assert_eq!(context.retained_bytes(), 0);
    assert!(later.is_empty());
}

#[cfg(feature = "raw")]
#[test]
fn pressure_observation_does_not_keep_cache_ownership_alive() {
    let (device, _, _) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 64,
        },
    );
    let values = context.materializations();
    let _watch = values.watch_pressure();
    values.set_retention_enabled(true);
    let image = produce(&values, 1, buffer(&device, 1.));
    let weak = Arc::downgrade(&image.image);
    drop(image);
    assert!(weak.upgrade().is_some());
    drop(values);
    assert!(weak.upgrade().is_some(), "the context still owns its store");
    drop(context);
    assert!(
        weak.upgrade().is_none(),
        "the observer has no cache ownership"
    );
}

#[test]
fn image_and_generated_value_lookups_update_the_common_recency_order() {
    let (device, _, _) = crate::gpu::test_support::render_context();
    for generated in [false, true] {
        let context = Context::new(
            &device,
            CachePolicy {
                generated_bytes: 128,
            },
        );
        let values = context.materializations();
        for key in [1u32, 2] {
            if generated {
                let value = values
                    .generated(&device, &RetainedKey::new(key), 64, true, true)
                    .unwrap();
                value.submitted();
            } else {
                drop(produce(&values, key, buffer(&device, key as f32)));
            }
        }
        assert_eq!(context.retained_bytes(), 128);
        let first = RetainedKey::new(1u32);
        if generated {
            let hit = values.generated(&device, &first, 64, true, false).unwrap();
            assert!(hit.ready());
        } else {
            assert_eq!(values.ready::<u32>(&first).len(), 1);
        }
        drop(produce(&values, 3, buffer(&device, 3.)));
        let second = RetainedKey::new(2u32);
        if generated {
            assert!(values.find::<GeneratedLayout>(&second, |_| true).is_none());
            assert!(values.find::<GeneratedLayout>(&first, |_| true).is_some());
        } else {
            assert!(values.find::<u32>(&second, |_| true).is_none());
            assert!(values.find::<u32>(&first, |_| true).is_some());
        }
        assert_eq!(context.retained_bytes(), 128);
    }
}

#[test]
fn retention_changes_preserve_joined_generated_values_without_restoring_optional_ownership() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    for pressure in [false, true] {
        let context = Context::new(
            &device,
            CachePolicy {
                generated_bytes: 64,
            },
        );
        let values = context.materializations();
        let key = RetainedKey::new("submitted generated samples");
        let generated = values.generated(&device, &key, 64, true, true).unwrap();
        let joined = values.find::<GeneratedLayout>(&key, |_| true).unwrap();
        assert_eq!(context.retained_bytes(), 64);
        if pressure {
            values.set_retention_enabled(false);
        } else {
            values.set_retention_limit(0);
        }
        assert_eq!(context.retained_bytes(), 0);
        queue.write_buffer(
            &generated.pixels,
            0,
            bytemuck::cast_slice(&[[2f32, -0.25, 3., 9.]; 4]),
        );
        queue.submit([]);
        generated.submitted();
        let image = pollster::block_on(joined).unwrap();
        let weak = Arc::downgrade(&image.image);
        assert_eq!(
            execute::read(&device, &queue, image.as_buffer().unwrap()),
            [[2., -0.25, 3., 9.]; 4]
        );
        assert_eq!(context.retained_bytes(), 0);
        drop((image, generated));
        assert!(weak.upgrade().is_none());
        assert!(values.is_empty());
        if pressure {
            values.set_retention_enabled(true);
        } else {
            values.set_retention_limit(64);
        }
        drop(produce(&values, 2, buffer(&device, 4.)));
        assert_eq!(context.retained_bytes(), 64);
        assert!(
            values
                .find::<u32>(&RetainedKey::new(2u32), |_| true)
                .is_some()
        );
    }
}

#[test]
fn disabled_or_busy_retention_does_not_hide_active_values_or_charge_joined_consumers_twice() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    for budget in [0, 63] {
        let context = Context::new(
            &device,
            CachePolicy {
                generated_bytes: budget,
            },
        );
        let scope = context.test_materializations(true);
        let active = produce(&scope, 1, buffer(&device, 2.));
        let joined = pollster::block_on(
            scope
                .find::<u32>(&RetainedKey::new(1u32), |_| true)
                .unwrap(),
        )
        .unwrap();
        assert!(Arc::ptr_eq(&active.image, &joined.image));
        assert_eq!(context.retained_bytes(), 0);
        assert_eq!(
            execute::read(&device, &queue, active.as_buffer().unwrap()),
            [[2., -0.25, 3., 9.]; 4]
        );
        drop((active, joined));
        assert!(
            scope
                .find::<u32>(&RetainedKey::new(1u32), |_| true)
                .is_none()
        );
    }
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 64,
        },
    );
    let first = context.test_materializations(true);
    let second = context.test_materializations(true);
    let active = produce(&first, 1, buffer(&device, 1.));
    let joined = pollster::block_on(
        first
            .find::<u32>(&RetainedKey::new(1u32), |_| true)
            .unwrap(),
    )
    .unwrap();
    assert!(Arc::ptr_eq(&active.image, &joined.image));
    assert_eq!(context.retained_bytes(), 64);
    let busy = produce(&second, 2, buffer(&device, 3.));
    assert!(
        second
            .find::<u32>(&RetainedKey::new(2u32), |_| true)
            .is_some()
    );
    assert_eq!(context.retained_bytes(), 64);
    assert!(!first.belongs_to(&Context::new(&device, Default::default())));
    drop(first);
    assert_eq!(context.retained_bytes(), 0);
    drop(busy);
    drop(produce(&second, 2, buffer(&device, 3.)));
    assert_eq!(context.retained_bytes(), 64);
    drop(second);
    assert_eq!(context.retained_bytes(), 0);
}

#[test]
fn batch_reservation_joins_existing_production_and_coalesces_duplicate_outputs() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (device, _, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.test_materializations(false);
    let pixels = buffer(&device, 1.);
    let counter = Arc::new(AtomicUsize::new(0));
    let calls = counter.clone();
    let existing = scope.obtain(
        RetainedKey::new(1u32),
        Arc::new(4u32),
        64,
        move || async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(GpuImage::new(pixels))
        },
    );
    let other = buffer(&device, 2.);
    let batch = scope.obtain_many(
        [1u32, 2, 2]
            .into_iter()
            .map(|key| (RetainedKey::new(key), Arc::new(4u32), 64))
            .collect(),
        move |missing| async move {
            assert_eq!(missing, vec![1]);
            Ok(vec![GpuImage::new(other)])
        },
    );
    drop(existing);
    let images = pollster::block_on(futures_util::future::try_join_all(batch)).unwrap();
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    assert!(Arc::ptr_eq(&images[1].image, &images[2].image));
    assert!(!Arc::ptr_eq(&images[0].image, &images[1].image));
    assert_eq!(context.retained_bytes(), 0);
}

#[test]
fn shared_awaitable_runs_one_production_and_survives_a_departing_consumer() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 128,
        },
    );
    let scope = context.test_materializations(true);
    let count = Arc::new(AtomicUsize::new(0));
    let calls = count.clone();
    let pixels = buffer(&device, 7.);
    let expected = pixels.clone();
    let first = scope.obtain(
        RetainedKey::new(9u32),
        Arc::new(4u32),
        pixels.size(),
        move || async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(GpuImage::new(pixels))
        },
    );
    let second = scope.obtain(RetainedKey::new(9u32), Arc::new(4u32), 64, || async {
        panic!("same value must join the existing computation")
    });
    assert_eq!(
        count.load(Ordering::SeqCst),
        0,
        "constructing the awaitable does not run GPU work on the caller"
    );
    drop(first);
    let image = pollster::block_on(second).unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(image.as_buffer(), Some(&expected));
    let cached = pollster::block_on(
        scope
            .find::<u32>(&RetainedKey::new(9u32), |size| *size == 4)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(cached.as_buffer(), image.as_buffer());
    scope.clear();
    assert_eq!(context.retained_bytes(), 0);
    assert_eq!(
        execute::read(&device, &queue, image.as_buffer().unwrap()),
        [[7., -0.25, 3., 9.]; 4]
    );
}

#[test]
fn failed_awaitables_can_retry_and_zero_retention_does_not_prevent_production() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 64,
        },
    );
    let scope = context.test_materializations(true);
    let failed = scope.obtain(RetainedKey::new(1u32), Arc::new(4u32), 64, || async {
        Err(Error::Cancelled)
    });
    assert!(pollster::block_on(failed).is_err());
    let pixels = buffer(&device, 3.);
    let retried = scope.obtain(
        RetainedKey::new(1u32),
        Arc::new(4u32),
        64,
        move || async move { Ok(GpuImage::new(pixels)) },
    );
    assert!(pollster::block_on(retried).is_ok());
    let disabled = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = disabled.test_materializations(true);
    let pixels = buffer(&device, 5.);
    let image = pollster::block_on(scope.obtain(
        RetainedKey::new(2u32),
        Arc::new(4u32),
        64,
        move || async move { Ok(GpuImage::new(pixels)) },
    ))
    .unwrap();
    assert_eq!(disabled.retained_bytes(), 0);
    let active = pollster::block_on(
        scope
            .find::<u32>(&RetainedKey::new(2u32), |_| true)
            .unwrap(),
    )
    .unwrap();
    assert!(
        Arc::ptr_eq(&active.image, &image.image),
        "active consumers share an image without optional ownership"
    );
    drop(active);
    assert_eq!(
        execute::read(&device, &queue, image.as_buffer().unwrap()),
        [[5., -0.25, 3., 9.]; 4]
    );
}

#[test]
fn in_flight_sharing_and_abandonment_do_not_depend_on_optional_retention() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (device, queue, _) = crate::gpu::test_support::render_context();
    for limit in [0, 64] {
        let context = Context::new(
            &device,
            CachePolicy {
                generated_bytes: limit,
            },
        );
        let scope = context.test_materializations(true);
        let pixels = buffer(&device, 6.);
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let (release, wait) = futures_channel::oneshot::channel();
        let mut first = scope.obtain(
            RetainedKey::new(11u32),
            Arc::new(4u32),
            64,
            move || async move {
                count.fetch_add(1, Ordering::SeqCst);
                wait.await.unwrap();
                Ok(GpuImage::new(pixels))
            },
        );
        assert!(pollster::block_on(futures_lite::future::poll_once(&mut first)).is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let second = scope.obtain(RetainedKey::new(11u32), Arc::new(4u32), 64, || async {
            panic!("an active requester already produces this image")
        });
        drop(first);
        scope.clear();
        release.send(()).unwrap();
        let image = pollster::block_on(second).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(
            scope.is_empty(),
            "clearing the lookup also suppresses late optional publication"
        );
        assert_eq!(context.retained_bytes(), 0);
        assert_eq!(
            execute::read(&device, &queue, image.as_buffer().unwrap()),
            [[6., -0.25, 3., 9.]; 4]
        );
        let abandoned = scope.obtain(RetainedKey::new(12u32), Arc::new(4u32), 64, || async {
            panic!("dropped unpolled production must never run")
        });
        drop(abandoned);
        assert!(
            scope.is_empty(),
            "weak lookup does not keep abandoned production alive"
        );
    }
}

#[test]
fn original_and_reconstruction_definitions_own_their_value_identity() {
    let file = crate::raw::fixture::bayer_dng();
    let scan = crate::scan::Scan::open(file.path(), Some([1.; 4])).unwrap();
    let first = super::super::original::camera_source(&scan).unwrap();
    let second = super::super::original::camera_source(&scan).unwrap();
    assert!(!Arc::ptr_eq(&first, &second));
    assert!(first.identity().is_some());
    assert_eq!(first.identity(), second.identity());
    let output = Domain {
        bounds: Some([0, 0, 4, 3].into()),
        grid: Grid::axis_aligned([1., 2.], [2., 3.]),
        meaning: Meaning::CameraCodes,
    };
    let reduced = super::super::reconstruction::reconstruct(first, output).unwrap();
    let same = super::super::reconstruction::reconstruct(second.clone(), output).unwrap();
    assert_eq!(reduced.identity(), same.identity());
    let shifted = super::super::reconstruction::reconstruct(
        second,
        Domain {
            grid: Grid::axis_aligned([1.25, 2.], [2., 3.]),
            ..output
        },
    )
    .unwrap();
    assert_ne!(
        reduced.identity(),
        shifted.identity(),
        "the aperture mapping belongs to the calculation, not cache bookkeeping"
    );
    assert!(
        borrowed_input(output).identity().is_none(),
        "an unbound execution input does not name immutable pixel content"
    );
}
