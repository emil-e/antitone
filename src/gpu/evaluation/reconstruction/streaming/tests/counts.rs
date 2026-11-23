use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[test]
fn sink_binds_an_exact_produced_input_without_a_transfer_allocation() {
    let (input, _) = fixture([64, 2048], vec![[2., -3., 4., 1.]; 64 * 2048]);
    let mapping = Mapping {
        start: [0.; 2],
        axes: [[64., 0.], [0., 2048.]],
    };
    let region = [0, 0, 1, 1].into();
    let domain = output(mapping, region);
    let producer = reconstruct_mapped(cpu_boundary(input), domain, mapping).unwrap();
    let identity = producer.output(region).unwrap().identity().unwrap().clone();
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let mut completed = Vec::new();

    for (direct, cancel) in [(false, false), (true, false), (true, true)] {
        let context = Context::new(&device, CachePolicy::default());
        let mut fragment = Fragment::default();
        // Hold the consumer and producer fixed; the ordinary scratch slot is
        // not a generated-image destination and must still be filled by copy.
        let input = value::image_resource(&mut fragment, &producer, region, direct);
        fragment.work.push(Work::Produce {
            image: producer.clone(),
            output: input,
        });
        let result = fragment.image(domain, region);
        fragment.work.push(Work::Copy {
            source: input.resource,
            target: result.resource,
            source_offset: 0,
            target_offset: 0,
            bytes: 16,
        });
        let plan = Plan::from_work(fragment, result).unwrap();
        let stopped = AtomicBool::new(false);
        let submissions = AtomicUsize::new(0);
        let scope = context.materializations();
        let mut executor = Executor::with_scope(context.programs.clone(), scope.clone());
        let pixels = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 16,
            usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let result = context
            .run_sink(
                &mut executor,
                &queue,
                &plan,
                (&|| stopped.load(Ordering::Relaxed), &|| {}),
                (
                    Sink {
                        encode: |encoder: &mut wgpu::CommandEncoder, _, output: &wgpu::Buffer| {
                            assert_eq!(
                                scope.ready::<ImageLayout>(&identity).len(),
                                1,
                                "only the independently submitted intermediate is ready"
                            );
                            encoder.copy_buffer_to_buffer(output, 0, &pixels, 0, 16);
                            stopped.store(cancel, Ordering::Relaxed);
                            Ok(())
                        },
                        submitted: || {
                            submissions.fetch_add(1, Ordering::Relaxed);
                        },
                    },
                    None,
                ),
            )
            .unwrap();
        assert_eq!(result.is_none(), cancel);
        assert_eq!(submissions.load(Ordering::Relaxed), usize::from(!cancel));
        assert_eq!(
            scope.ready::<ImageLayout>(&identity).len(),
            1,
            "there must not be a second publication from a transfer destination"
        );
        if let Some(counts) = result {
            assert_eq!(
                super::super::super::super::execute::read(&device, &queue, &pixels),
                [[2., -3., 4., 9.]]
            );
            completed.push(counts);
        }
    }
    assert_eq!(
        completed[0].buffer_allocations,
        completed[1].buffer_allocations + 1
    );
    assert_eq!(completed[0].submissions, completed[1].submissions + 1);
}

#[test]
fn nested_production_counts_source_work_submissions_and_allocations() {
    let (input, reads) = fixture([512, 2560], vec![[2., -3., 4., 1.]; 512 * 2560]);
    let mapping = Mapping {
        start: [0.; 2],
        axes: [[512., 0.], [0., 2560.]],
    };
    let inner = reconstruct_mapped(
        cpu_boundary(input),
        output(mapping, [0, 0, 1, 1].into()),
        mapping,
    )
    .unwrap();
    let root = Value::from_operation(inner.domain, Some(inner), Square);
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let budget = Tiling { tile: [1, 1] };
    let evaluation = context
        .request(
            root.clone(),
            Region {
                bounds: root.domain.finite().unwrap(),
                grid: root.domain.grid,
            },
            budget,
        )
        .unwrap();
    let plan = evaluation.plans().next().unwrap().unwrap();
    assert!(
        plan.tiles
            .iter()
            .flat_map(|tile| &tile.fragment.work)
            .any(|work| matches!(work, Work::Produce { .. }))
    );

    for delivery in ["read", "sink", "production"] {
        reads.lock().unwrap().clear();
        let gate = queue.clone();
        let submissions = Arc::new(AtomicUsize::new(0));
        let observed = submissions.clone();
        gate.observe_submissions(Arc::new(move || {
            observed.fetch_add(1, Ordering::Relaxed);
        }));
        let pixels = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 16,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        crate::gpu::test_support::take_evaluation_buffer_creations();
        let mut actual = Vec::new();
        let stats = match delivery {
            "read" => context
                .read(
                    &queue,
                    &evaluation,
                    || false,
                    |_, values| {
                        actual.extend_from_slice(values);
                        Ok(())
                    },
                )
                .unwrap()
                .unwrap(),
            "sink" => context
                .evaluate_with(
                    &queue,
                    &evaluation,
                    || false,
                    Sink {
                        encode: |encoder: &mut wgpu::CommandEncoder,
                                 _: PlannedImage,
                                 input: &wgpu::Buffer| {
                            encoder.copy_buffer_to_buffer(input, 0, &pixels, 0, 16);
                            Ok(())
                        },
                        submitted: || {},
                    },
                )
                .unwrap()
                .unwrap(),
            _ => Production {
                worker: Worker {
                    context: &context,
                    queue: &queue,
                    scope: &context.materializations(),
                    working: &RunWorkingSet::default(),
                    cancelled: &|| false,
                    progress: &|| {},
                },
                layout: ImageLayout {
                    domain: root.domain,
                    region: root.domain.finite().unwrap(),
                    format: ImageFormat::Rgba32Float,
                    backing: ImageBacking::Buffer,
                },
                pixels: &pixels,
            }
            .produce(&evaluation)
            .unwrap(),
        };
        let [buffers, _] = crate::gpu::test_support::take_evaluation_buffer_creations();
        assert_eq!(
            stats.submissions,
            submissions.load(Ordering::Relaxed),
            "{delivery}"
        );
        assert!(
            stats.submissions > 3,
            "include contributions and clear/final readouts"
        );
        assert_eq!(
            stats.buffer_allocations as u64, buffers,
            "{delivery}: include captures, reduction storage and the host readback"
        );
        let reads = reads.lock().unwrap();
        let cells = |region: &Rect| u64::from(region.width) * u64::from(region.height);
        assert!(reads.len() > 1);
        assert_eq!(
            stats.source_cells,
            reads.iter().map(cells).sum::<u64>(),
            "{delivery}"
        );
        assert_eq!(stats.source_cells, 512 * 2560);
        drop(reads);
        if delivery != "read" {
            actual = super::super::super::super::execute::read(&device, &queue, &pixels);
        }
        assert_eq!(actual, vec![[4., 9., 16., 9.]]);
    }
}
