use super::*;

struct Leaf(u32);
impl Operation for Leaf {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(self.0))
    }
    fn reads(&self, _: CellRegion) -> Result<Vec<Read>, Error> {
        Ok(Vec::new())
    }
}
struct Forward;
impl Operation for Forward {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("forward relevance fixture"))
    }
    fn reads(&self, output: CellRegion) -> Result<Vec<Read>, Error> {
        Ok(vec![Read::new(0, output)])
    }
}
fn domain() -> Domain {
    Domain {
        bounds: Some(CellRegion::new([0, 0], [8, 1])),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Neutral,
    }
}
fn value(id: u32) -> Value {
    Value::from_operation(domain(), [], Leaf(id))
}
fn store(bytes: u64) -> Materializations {
    Materializations::new(Arc::default(), cache::Allowance::new(bytes))
}
fn publish(scope: &Materializations, value: &Value, region: CellRegion) -> Image<ImageLayout> {
    let image = Image {
        identity: value.output(region).unwrap().identity().unwrap().clone(),
        descriptor: Arc::new(ImageLayout {
            domain: value.domain(),
            region,
            format: ImageFormat::Rgba32Float,
            backing: ImageBacking::Host,
        }),
        image: ImageValue::host(vec![
            [3., -2., 1., 0.];
            (region.width * region.height) as usize
        ]),
    };
    scope.publish(&image, true);
    image
}
fn retained(scope: &Materializations, value: &Value, region: CellRegion) -> bool {
    let key = value.output(region).unwrap().identity().unwrap().clone();
    scope.0.entries.lock().unwrap().iter().any(|entry| {
        entry.key == key
            && entry
                .descriptor
                .downcast_ref::<ImageLayout>()
                .is_some_and(|l| l.region == region)
            && entry.retained.is_some()
    })
}

#[test]
fn cached_descendant_demand_refreshes_its_upstream_without_pinning_or_extra_storage() {
    let scope = store(192);
    let region = CellRegion::new([0, 0], [4, 1]);
    let upstream = value(1);
    let unrelated = value(2);
    let root = Value::from_operation(domain(), [upstream.clone()], Forward);
    drop(publish(&scope, &upstream, region));
    drop(publish(&scope, &unrelated, region));
    drop(publish(&scope, &root, region));
    assert_eq!(scope.0.retention.bytes(), 192);

    scope.requested(&root, region);
    drop(publish(&scope, &value(3), region));
    assert!(retained(&scope, &upstream, region));
    assert!(retained(&scope, &root, region));
    assert!(!retained(&scope, &unrelated, region));
    assert_eq!(scope.0.retention.bytes(), 192);

    // New independent demand can still displace this graph; relevance is not a lease.
    drop(publish(&scope, &value(4), region));
    assert!(!retained(&scope, &upstream, region));
}

#[test]
fn regional_demand_does_not_refresh_disjoint_layouts_of_the_same_content() {
    let scope = store(128);
    let left = CellRegion::new([0, 0], [4, 1]);
    let right = CellRegion::new([4, 0], [4, 1]);
    let upstream = value(1);
    let root = Value::from_operation(domain(), [upstream.clone()], Forward);
    drop(publish(&scope, &upstream, left));
    drop(publish(&scope, &upstream, right));
    scope.requested(&root, left);
    drop(publish(&scope, &value(2), left));
    assert!(retained(&scope, &upstream, left));
    assert!(!retained(&scope, &upstream, right));
}

#[test]
fn changed_upstream_identity_does_not_refresh_the_previous_calculation() {
    let scope = store(128);
    let region = CellRegion::new([0, 0], [4, 1]);
    let old = value(1);
    let unrelated = value(2);
    drop(publish(&scope, &old, region));
    drop(publish(&scope, &unrelated, region));
    let root = Value::from_operation(domain(), [value(3)], Forward);
    scope.requested(&root, region);
    drop(publish(&scope, &value(4), region));
    assert!(!retained(&scope, &old, region));
    assert!(retained(&scope, &unrelated, region));
}

#[test]
fn discovery_does_not_promote_candidates_but_selected_use_does() {
    let region = CellRegion::new([0, 0], [4, 1]);
    for use_selected in [false, true] {
        let scope = store(128);
        let first = value(1);
        let second = value(2);
        drop(publish(&scope, &first, region));
        drop(publish(&scope, &second, region));
        let key = first.output(region).unwrap().identity().unwrap().clone();
        let candidates = scope.ready::<ImageLayout>(&key);
        assert_eq!(candidates.len(), 1);
        if use_selected {
            scope.used(&candidates[0].image);
        }
        drop(candidates);
        drop(publish(&scope, &value(3), region));
        assert_eq!(retained(&scope, &first, region), use_selected);
        assert_eq!(retained(&scope, &second, region), !use_selected);
    }
}

#[test]
fn reading_a_cached_descendant_refreshes_the_declared_upstream_demand() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 192,
        },
    );
    let scope = context.materializations();
    scope.set_test_pressure(crate::sys::MemoryPressure::Normal);
    let region = CellRegion::new([0, 0], [4, 1]);
    let upstream = value(1);
    let unrelated = value(2);
    let root = Value::from_operation(domain(), [upstream.clone()], Forward);
    drop(publish(&scope, &upstream, region));
    drop(publish(&scope, &unrelated, region));
    drop(publish(&scope, &root, region));
    let evaluation = context
        .request(
            root,
            Region {
                bounds: region,
                grid: domain().grid,
            },
            Budget {
                tile: [4, 1],
                scratch_bytes: 1 << 20,
            },
        )
        .unwrap();
    let mut blocks = 0;
    context
        .read(
            &queue,
            &Default::default(),
            &evaluation,
            || false,
            |block, pixels| {
                assert_eq!(block, region);
                assert_eq!(pixels, &[[3., -2., 1., 0.]; 4]);
                blocks += 1;
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(blocks, 1);
    drop(publish(&scope, &value(3), region));
    assert!(retained(&scope, &upstream, region));
    assert!(!retained(&scope, &unrelated, region));
}

#[test]
fn evaluating_a_cached_descendant_into_a_sink_refreshes_upstream_relevance() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 192,
        },
    );
    let scope = context.materializations();
    scope.set_test_pressure(crate::sys::MemoryPressure::Normal);
    let region = CellRegion::new([0, 0], [4, 1]);
    let upstream = value(1);
    let unrelated = value(2);
    let root = Value::from_operation(domain(), [upstream.clone()], Forward);
    drop(publish(&scope, &upstream, region));
    drop(publish(&scope, &unrelated, region));
    let image = Image {
        identity: root.output(region).unwrap().identity().unwrap().clone(),
        descriptor: Arc::new(ImageLayout {
            domain: domain(),
            region,
            format: ImageFormat::Rgba32Float,
            backing: ImageBacking::Buffer,
        }),
        image: ImageValue::new(buffer(&device, 3.)),
    };
    scope.publish(&image, true);
    drop(image);
    let evaluation = context
        .request(
            root,
            Region {
                bounds: region,
                grid: domain().grid,
            },
            Budget {
                tile: [4, 1],
                scratch_bytes: 1 << 20,
            },
        )
        .unwrap();
    let mut delivered = 0;
    let counts = context
        .evaluate(
            &queue,
            &Default::default(),
            &evaluation,
            || false,
            |_, image, _| {
                assert_eq!(image.region, region);
                delivered += 1;
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(delivered, 1);
    assert_eq!(counts.source_cells, 0);
    drop(publish(&scope, &value(3), region));
    assert!(retained(&scope, &upstream, region));
    assert!(!retained(&scope, &unrelated, region));
}

#[test]
fn recorded_demand_and_selected_storage_promote_only_after_submission() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let region = CellRegion::new([0, 0], [4, 1]);
    for submit in [false, true] {
        let context = Context::new(
            &device,
            CachePolicy {
                generated_bytes: 192,
            },
        );
        let scope = context.materializations();
        scope.set_test_pressure(crate::sys::MemoryPressure::Normal);
        let upstream = value(1);
        let unrelated = value(2);
        let root = Value::from_operation(domain(), [upstream.clone()], Forward);
        drop(publish(&scope, &upstream, region));
        drop(publish(&scope, &unrelated, region));
        let image = Image {
            identity: root.output(region).unwrap().identity().unwrap().clone(),
            descriptor: Arc::new(ImageLayout {
                domain: domain(),
                region,
                format: ImageFormat::Rgba32Float,
                backing: ImageBacking::Buffer,
            }),
            image: ImageValue::new(buffer(&device, 3.)),
        };
        scope.publish(&image, true);
        drop(image);
        let evaluation = context
            .request(
                root,
                Region {
                    bounds: region,
                    grid: domain().grid,
                },
                Budget {
                    tile: [4, 1],
                    scratch_bytes: 1 << 20,
                },
            )
            .unwrap();
        // Even a successful probe does not establish submitted use.
        Job::new(
            evaluation.image().clone(),
            region,
            Budget {
                tile: [4, 1],
                scratch_bytes: 1 << 20,
            },
        )
        .unwrap()
        .with_materializations(scope.clone())
        .probe(region)
        .unwrap();
        let recorded = context.record(evaluation, scope.clone(), vec![]).unwrap();
        if submit {
            let output = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 64,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            recorded
                .encode_result(&device, &mut encoder, &output)
                .unwrap();
            context
                .submit_with(
                    &queue,
                    &Default::default(),
                    encoder,
                    || false,
                    || recorded.submitted(),
                )
                .unwrap()
                .unwrap();
            assert_eq!(
                execute::read(&device, &queue, &output),
                [[3., -0.25, 3., 9.]; 4]
            );
        }
        drop(publish(&scope, &value(3), region));
        assert_eq!(retained(&scope, &upstream, region), submit);
        assert_eq!(retained(&scope, &unrelated, region), !submit);
        assert_eq!(scope.0.retention.bytes(), 192);
    }
}

#[test]
fn relevance_cannot_restore_disabled_retention_or_invalidate_an_independent_consumer() {
    let scope = store(64);
    let region = CellRegion::new([0, 0], [4, 1]);
    let upstream = value(1);
    let image = publish(&scope, &upstream, region);
    let weak = Arc::downgrade(&image.image);
    let root = Value::from_operation(domain(), [upstream.clone()], Forward);
    scope.set_retention_enabled(false);
    scope.requested(&root, region);
    scope.used(&image.image);
    assert_eq!(scope.0.retention.bytes(), 0);
    assert!(!retained(&scope, &upstream, region));
    assert_eq!(image.as_host().unwrap(), &[[3., -2., 1., 0.]; 4]);
    drop(image);
    assert!(weak.upgrade().is_none());
    assert_eq!(scope.0.retention.bytes(), 0);
}
