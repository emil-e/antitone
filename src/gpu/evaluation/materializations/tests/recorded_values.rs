use super::*;

fn recording_values() -> (Value, Value, Domain, CellRegion) {
    let region = CellRegion::new([0, 0], [4, 1]);
    let domain = Domain {
        bounds: Some(region),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Arguments,
    };
    let seed = value::Value::new(
        domain,
        region,
        vec![],
        value::Operation::new(
            "recording constant pixels",
            (),
            "fn constant(p:vec2i,at:vec2f)->vec4f{return vec4f(2.,-0.25,3.,9.);}",
            "constant",
            value::Access::Pointwise,
        ),
    )
    .unwrap();
    let seed = seed;
    let root = value::pointwise(
        seed.clone(),
        Meaning::Arguments,
        value::Operation::new(
            "recording doubled pixels",
            (),
            "fn double(p:vec2i,at:vec2f,pixel:vec4f)->vec4f{return vec4f(pixel.rgb*2.,pixel.a);}",
            "double",
            value::Access::Pointwise,
        ),
    )
    .unwrap();

    (seed, root, domain, region)
}

#[test]
fn recorded_value_images_publish_after_commit_without_blocking_independent_producers() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    for capacity in [0, 128] {
        let context = Context::new(
            &device,
            CachePolicy {
                generated_bytes: capacity,
            },
        );
        let scope = context.materializations();
        let domain = Domain {
            bounds: Some(CellRegion::new([0, 0], [4, 4])),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::Arguments,
        };
        let layout = ImageLayout {
            domain,
            region: CellRegion::new([0, 0], [4, 1]),
            format: ImageFormat::Rgba32Float,
            backing: ImageBacking::Buffer,
        };
        let key = RetainedKey::new("constant recorded image witness");
        let recorded = scope
            .generated_image(&device, &key, layout, true, true)
            .unwrap();
        assert!(!recorded.ready());
        assert!(
            scope
                .find::<ImageLayout>(&key, |value| value == &layout)
                .is_none(),
            "a recording cannot make a worker await an unsubmitted encoder"
        );
        let independent_pixels = buffer(&device, 2.);
        let value=scope.obtain(key.clone(),Arc::new(layout),layout.bytes(),move||async move{Ok(ImageValue::new(independent_pixels))}).now_or_never().expect("the independently requested calculation must run before the recording is committed").unwrap();
        assert_ne!(
            value.as_buffer().unwrap(),
            &recorded.pixels,
            "the control needs genuinely independent physical production"
        );
        queue.write_buffer(
            &recorded.pixels,
            0,
            bytemuck::cast_slice(&[[2f32, -0.25, 3., 9.]; 4]),
        );
        queue.submit([]);
        recorded.submitted();
        assert!(recorded.ready());
        let wrong = ImageLayout {
            region: CellRegion::new([0, 0], [2, 2]),
            ..layout
        };
        assert!(
            scope
                .find::<ImageLayout>(&key, |value| value == &wrong)
                .is_none(),
            "equal byte counts do not make different image layouts interchangeable"
        );
        let ready = pollster::block_on(
            scope
                .find::<ImageLayout>(&key, |value| value == &layout)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            execute::read(&device, &queue, ready.as_buffer().unwrap()),
            [[2., -0.25, 3., 9.]; 4]
        );
        scope.set_retention_limit(0);
        assert_eq!(context.retained_bytes(), 0);
        let weak = Arc::downgrade(&ready.image);
        drop((ready, value, recorded));
        assert!(weak.upgrade().is_none());
        assert!(
            scope.is_empty(),
            "completed image discovery must not retain abandoned live owners"
        );
    }
}

#[test]
fn cold_recorded_calculation_keeps_intermediate_publication_through_real_zero_retention_submission()
{
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.materializations();
    let (seed, root, domain, region) = recording_values();
    let key = seed.identity().unwrap().clone();
    let evaluation = context
        .request(
            root,
            Region {
                bounds: region,
                grid: domain.grid,
            },
            Budget {
                tile: [4, 1],
                scratch_bytes: 1 << 20,
            },
        )
        .unwrap()
        .with_policy(Policy::Materialized);
    let recorded = context.record(evaluation, scope.clone(), vec![]).unwrap();
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 64,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut first = None;
    for _ in 0..2 {
        let mut encoder = device.create_command_encoder(&Default::default());
        let counts = recorded
            .encode_result_counts(&device, &mut encoder, &output)
            .unwrap();
        if first.is_none() {
            assert!(scope.find::<ImageLayout>(&key, |_| true).is_none());
        }
        queue.submit([encoder.finish()]);
        recorded.submitted();
        let ready = pollster::block_on(
            scope
                .find::<ImageLayout>(&key, |layout| layout.region == region)
                .expect(
                    "submitted live intermediate must be discoverable without optional retention",
                ),
        )
        .unwrap();
        assert_eq!(
            execute::read(&device, &queue, ready.as_buffer().unwrap()),
            [[2., -0.25, 3., 9.]; 4]
        );
        assert_eq!(
            execute::read(&device, &queue, &output),
            [[4., -0.5, 6., 9.]; 4]
        );
        if let Some(initializations) = first {
            assert!(
                counts.initializations < initializations,
                "later encodings must consume the initialized image"
            );
        } else {
            assert!(counts.initializations > 0);
            first = Some(counts.initializations);
        }
        assert_eq!(context.retained_bytes(), 0);
    }
    drop(recorded);
    assert!(
        scope.find::<ImageLayout>(&key, |_| true).is_none(),
        "the store cannot keep discarded live owners alive"
    );
}

#[test]
fn live_compiler_values_become_optional_retention_only_through_an_image_demand() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 4096,
        },
    );
    let scope = context.materializations();
    let (seed, root, domain, region) = recording_values();
    let key = seed.identity().unwrap().clone();
    let request = |image| {
        context
            .request(
                image,
                Region {
                    bounds: region,
                    grid: domain.grid,
                },
                Budget {
                    tile: [4, 1],
                    scratch_bytes: 1 << 20,
                },
            )
            .unwrap()
    };
    let recorded = context
        .record(
            request(root).with_policy(Policy::Materialized),
            scope.clone(),
            vec![],
        )
        .unwrap();
    assert_eq!(
        context.retained_bytes(),
        0,
        "recording a consumer must not select all compiler temporaries for retention"
    );
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 64,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    recorded
        .encode_result(&device, &mut encoder, &output)
        .unwrap();
    queue.submit([encoder.finish()]);
    recorded.submitted();
    let live = pollster::block_on(scope.find::<ImageLayout>(&key, |_| true).unwrap()).unwrap();
    assert_eq!(
        context.retained_bytes(),
        0,
        "live discovery does not imply optional retention"
    );
    let selected = pollster::block_on(
        context
            .materialize(
                &queue,
                &Default::default(),
                &scope,
                request(seed),
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap();
    assert!(
        Arc::ptr_eq(&live.image, &selected.image),
        "an ordinary image request must be able to select the already-live value"
    );
    assert_eq!(
        execute::read(&device, &queue, selected.as_buffer().unwrap()),
        [[2., -0.25, 3., 9.]; 4]
    );
    assert_eq!(context.retained_bytes(), 64);
    drop((recorded, live, selected));
    assert!(scope.find_ready::<ImageLayout>(&key, |_| true).is_some());
    scope.set_retention_limit(0);
    assert!(scope.find_ready::<ImageLayout>(&key, |_| true).is_none());
}
