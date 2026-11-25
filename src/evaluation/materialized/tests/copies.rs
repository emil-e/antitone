use super::*;
use crate::evaluation::context::{Production, Worker};
use wgpu::util::DeviceExt;

fn domain() -> Domain {
    Domain {
        bounds: Some([-8, -8, 24, 24].into()),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
    }
}
fn wanted() -> CellRegion {
    [-3, -2, 10, 9].into()
}
fn field(region: CellRegion) -> Vec<[f32; 4]> {
    (region.y..region.bottom())
        .flat_map(|y| {
            (region.x..region.right()).map(move |x| {
                [
                    -(x as f32) - 0.25,
                    16. + y as f32,
                    f32::from_bits(0x0000_0011),
                    ((x - y).rem_euclid(16)) as f32,
                ]
            })
        })
        .collect()
}
struct Unreadable(Arc<AtomicUsize>);
impl Source for Unreadable {
    fn domain(&self) -> Domain {
        domain()
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("covered source field"))
    }
    fn read(&self, _: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Err(Error::InvalidParameter("covered source was read again"))
    }
}
struct Uncomputed(Arc<AtomicUsize>);
impl Operation for Uncomputed {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("covered generated field"))
    }
    fn calculation(
        self: Arc<Self>,
        _: &[Value],
        domain: Domain,
        region: CellRegion,
        _: Tiling,
    ) -> Result<Option<value::Calculation>, Error> {
        Ok(Some(value::Calculation::new(
            domain,
            region,
            Vec::new(),
            value::Operation::cpu("covered generated field", self.key(), self),
        )?))
    }
}
impl CpuKernel for Uncomputed {
    fn run(
        &self,
        _: &[CpuImage],
        _: CellRegion,
        _: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<[f32; 4]>>, Error> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Err(Error::InvalidParameter(
            "covered generated field was recomputed",
        ))
    }
}
fn publish(context: &Context, root: &Value, region: CellRegion, transfer: bool) -> Arc<ImageValue> {
    let pixels = ImageValue::new(context.device().create_buffer_init(
        &wgpu::util::BufferInitDescriptor {
            label: Some("cached fragment"),
            contents: bytemuck::cast_slice(&field(region)),
            usage: wgpu::BufferUsages::STORAGE
                | if transfer {
                    wgpu::BufferUsages::COPY_SRC
                } else {
                    wgpu::BufferUsages::empty()
                },
        },
    ));
    context.materializations().publish(
        &Image {
            identity: root.output(region).unwrap().identity().unwrap().clone(),
            descriptor: Arc::new(ImageLayout {
                domain: domain(),
                region,
                format: ImageFormat::Rgba32Float,
                backing: ImageBacking::Buffer,
            }),
            image: pixels.clone(),
        },
        false,
    );
    pixels
}
fn pieces(
    context: &Context,
    root: &Value,
    omitted: Option<usize>,
    transfer: bool,
) -> Vec<Arc<ImageValue>> {
    (0..9)
        .filter(|i| Some(*i) != omitted)
        .map(|i| {
            let region: CellRegion =
                [-4 + (i % 3) as i32 * 4, -4 + (i / 3) as i32 * 4, 4, 4].into();
            publish(context, root, region, transfer)
        })
        .collect()
}
fn query(context: &Context, root: Value) -> Evaluation {
    context
        .request(
            root,
            Region {
                bounds: wanted(),
                grid: domain().grid,
            },
            Tiling { tile: [10, 9] },
        )
        .unwrap()
        .without_retention()
}
fn output(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("existing covered output"),
        size: 10 * 9 * 16,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
fn bits(actual: &[[f32; 4]], expected: &[[f32; 4]]) {
    assert_eq!(
        bytemuck::cast_slice::<_, u8>(actual),
        bytemuck::cast_slice::<_, u8>(expected)
    );
}

#[test]
fn frozen_preparation_uses_nine_late_live_fragments_without_source_reads() {
    let (device, queue, _) = crate::photographic::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let calls = Arc::new(AtomicUsize::new(0));
    let root = source(Arc::new(Unreadable(calls.clone())));
    let request = query(&context, root.clone());
    let preparation = context.prepare_image(request).unwrap();
    assert_eq!(
        calls.load(Ordering::Relaxed),
        0,
        "preparation must not acquire source pixels"
    );
    let held = pieces(&context, &root, None, true);
    assert_eq!(held.len(), 9);
    assert!(
        Stored::find(Some(&context.materializations()), &[], &root, wanted())
            .unwrap()
            .is_none(),
        "the four-binding route must genuinely miss this fixture"
    );
    let image = context
        .produce_prepared(&queue, preparation, || false)
        .unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    bits(
        &execute::read(&device, &queue, image.image.as_buffer().unwrap()),
        &field(wanted()),
    );
    assert_eq!(context.retained_bytes(), 0);
}

#[test]
fn generated_values_copy_into_existing_output_and_own_fragments_until_gpu_completion() {
    let (device, queue, _) = crate::photographic::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.materializations();
    let calls = Arc::new(AtomicUsize::new(0));
    let root = Value::from_operation(domain(), [], Uncomputed(calls.clone()));
    let held = pieces(&context, &root, None, true);
    let weak: Vec<_> = held.iter().map(Arc::downgrade).collect();
    let output = output(&device);
    let working = RunWorkingSet::default();
    let production = Production {
        worker: Worker {
            context: &context,
            queue: &queue,
            scope: &scope,
            working: &working,
            cancelled: &|| false,
            progress: &|| {},
        },
        layout: ImageLayout {
            domain: domain(),
            region: wanted(),
            format: ImageFormat::Rgba32Float,
            backing: ImageBacking::Buffer,
        },
        pixels: &output,
    };
    let counts = production.produce(&query(&context, root)).unwrap();
    assert!(counts.submissions > 0);
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    drop(held);
    assert!(
        weak.iter().all(|image| image.upgrade().is_some()),
        "queued copy groups own every selected physical fragment"
    );
    working.finish_gpu().unwrap();
    assert!(
        weak.iter().all(|image| image.upgrade().is_none()),
        "finished copies need no frame-owned source pins"
    );
    bits(&execute::read(&device, &queue, &output), &field(wanted()));
}

#[test]
fn copy_coverage_rejects_holes_nontransferable_storage_and_excessive_commands() {
    let (device, _, _) = crate::photographic::test_support::render_context();
    for (omitted, transfer, commands) in [
        (Some(4), true, value::MAX_UNIT_VALUES),
        (None, false, value::MAX_UNIT_VALUES),
        (None, true, 1),
    ] {
        let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
        let root = source(Arc::new(Unreadable(Arc::default())));
        let _held = pieces(&context, &root, omitted, transfer);
        let mut remaining = value::MAX_UNIT_VALUES;
        assert!(
            super::super::CopyCover::find(
                &context.materializations(),
                &root,
                wanted(),
                commands,
                &mut remaining
            )
            .unwrap()
            .is_none()
        );
    }
}

#[test]
fn overlapping_support_clips_to_disjoint_copies_without_changing_pixel_bits() {
    let (device, queue, _) = crate::photographic::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let root = source(Arc::new(Unreadable(Arc::default())));
    let mut held = pieces(&context, &root, None, true);
    held.push(publish(&context, &root, [-5, -5, 6, 14].into(), true));
    let mut remaining = value::MAX_UNIT_VALUES;
    let cover = super::super::CopyCover::find(
        &context.materializations(),
        &root,
        wanted(),
        value::MAX_UNIT_VALUES,
        &mut remaining,
    )
    .unwrap()
    .unwrap();
    let area: u64 = cover
        .regions
        .iter()
        .map(|part| u64::from(part.region.width) * u64::from(part.region.height))
        .sum();
    assert_eq!(
        area, 90,
        "overlapping storage must not produce overlapping writes"
    );
    let pixels = output(&device);
    let mut encoder = device.create_command_encoder(&Default::default());
    cover.encode(&mut encoder, &pixels).unwrap();
    context.submit(&queue, encoder, || false).unwrap().unwrap();
    bits(&execute::read(&device, &queue, &pixels), &field(wanted()));
}

#[test]
fn incomplete_coverage_falls_back_and_cancellation_cannot_publish_a_copied_result() {
    let (device, queue, _) = crate::photographic::test_support::render_context();
    for cancel in [false, true] {
        let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
        let calls = Arc::new(AtomicUsize::new(0));
        let root = source(Arc::new(Unreadable(calls.clone())));
        let preparation = context
            .prepare_image(query(&context, root.clone()))
            .unwrap();
        let _held = pieces(&context, &root, if cancel { None } else { Some(4) }, true);
        let result = context.produce_prepared(&queue, preparation, || cancel);
        if cancel {
            assert!(matches!(result, Err(Error::Cancelled)));
            assert_eq!(calls.load(Ordering::Relaxed), 0);
        } else {
            assert!(matches!(
                result,
                Err(Error::InvalidParameter("covered source was read again"))
            ));
            assert!(
                calls.load(Ordering::Relaxed) > 0,
                "a hole must actually take the producer fallback"
            );
        }
        let full = ImageLayout {
            domain: domain(),
            region: wanted(),
            format: ImageFormat::Rgba32Float,
            backing: ImageBacking::Buffer,
        };
        assert!(
            context
                .materializations()
                .ready::<ImageLayout>(root.output(wanted()).unwrap().identity().unwrap())
                .iter()
                .all(|image| *image.descriptor != full),
            "failed or cancelled work must not publish a complete image"
        );
    }
}

#[test]
fn copy_discovery_spends_one_shared_budget_and_declines_incomplete_scans() {
    let (device, _, _) = crate::photographic::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.materializations();
    let root = source(Arc::new(Unreadable(Arc::default())));
    let _held = pieces(&context, &root, None, true);
    let mut remaining = 4;
    assert!(
        super::super::CopyCover::find(
            &scope,
            &root,
            wanted(),
            value::MAX_UNIT_VALUES,
            &mut remaining
        )
        .unwrap()
        .is_none()
    );
    assert_eq!(remaining, 0);
    assert!(
        scope
            .ready_bounded::<ImageLayout>(
                root.output(wanted()).unwrap().identity().unwrap(),
                &mut remaining
            )
            .is_empty()
    );
}

#[test]
fn frozen_upload_reuse_preserves_queued_inputs_after_cancellation_on_submission() {
    use std::sync::atomic::AtomicBool;
    let (device, queue, _) = crate::photographic::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.materializations();
    let calls = Arc::new(AtomicUsize::new(0));
    let root = source(Arc::new(Unreadable(calls.clone())));
    let budget = Tiling { tile: [10, 9] };
    let second: CellRegion = [-2, -1, 9, 8].into();
    let plan = |region| {
        Job::new(root.clone(), region, budget)
            .unwrap()
            .with_limits(device.limits())
            .with_materializations(scope.clone())
            .probe(region)
            .unwrap()
    };
    let first_plan = plan(wanted());
    let second_plan = plan(second);
    let held = pieces(&context, &root, None, true);
    let inputs: Vec<_> = held.iter().map(Arc::downgrade).collect();
    let held = Arc::new(std::sync::Mutex::new(Some(held)));
    let first_pixels = output(&device);
    let second_pixels = output(&device);
    let gate = queue.clone();
    let working = RunWorkingSet::default();
    let stop = Arc::new(AtomicBool::new(false));
    let cancelled = || stop.load(Ordering::Relaxed);
    let worker = Worker {
        context: &context,
        queue: &queue,
        scope: &scope,
        working: &working,
        cancelled: &cancelled,
        progress: &|| {},
    };
    let production = |region, pixels| Production {
        worker,
        layout: ImageLayout {
            domain: domain(),
            region,
            format: ImageFormat::Rgba32Float,
            backing: ImageBacking::Buffer,
        },
        pixels,
    };
    let mut executor = worker.executor();
    production(wanted(), &first_pixels)
        .deliver(&mut executor, first_plan)
        .unwrap();
    // Separate the second submission's ownership from the first completed group.
    working.finish_gpu().unwrap();
    let observed_inputs = inputs.clone();
    let observed_stop = stop.clone();
    gate.observe_submissions(Arc::new(move || {
        held.lock().unwrap().take();
        assert!(
            observed_inputs
                .iter()
                .all(|image| image.upgrade().is_some()),
            "second submission owns its cached input"
        );
        observed_stop.store(true, Ordering::Relaxed);
    }));
    assert!(matches!(
        production(second, &second_pixels).deliver(&mut executor, second_plan),
        Err(Error::Cancelled)
    ));
    drop(executor);
    assert!(
        inputs.iter().all(|image| image.upgrade().is_some()),
        "clearing and dropping executor state must not release queued storage"
    );
    working.finish_gpu().unwrap();
    assert!(inputs.iter().all(|image| image.upgrade().is_none()));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    bits(
        &execute::read(&device, &queue, &first_pixels),
        &field(wanted()),
    );
    bits(
        &execute::read(&device, &queue, &second_pixels)[..72],
        &field(second),
    );
}
