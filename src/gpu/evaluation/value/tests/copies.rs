use super::*;

#[test]
fn partial_external_arguments_need_only_storage_and_preserve_all_sample_bits() {
    use wgpu::util::DeviceExt;
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let bounds = CellRegion::new([-7, 11], [6, 4]);
    let domain = Domain {
        bounds: Some(bounds),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Neutral,
    };
    let original: Vec<[u32; 4]> = (0..24u32)
        .map(|i| {
            [
                if i % 2 == 0 {
                    0x8000_0000
                } else {
                    0x7fc0_0123 + i
                },
                (i as f32 * -3.5).to_bits(),
                1 + i,
                (i as f32 * 0.25 - 2.).to_bits(),
            ]
        })
        .collect();
    let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("storage-only borrowed samples"),
        contents: bytemuck::cast_slice(&original),
        usage: wgpu::BufferUsages::STORAGE,
    });
    assert_eq!(input.usage(), wgpu::BufferUsages::STORAGE);
    let root = super::super::super::resident::borrowed_input(domain);
    for policy in [Policy::Materialized, Policy::Composed] {
        for region in [
            CellRegion::new([-6, 12], [3, 2]),
            CellRegion::new([-5, 12], [3, 2]),
            CellRegion::new([-6, 12], [3, 2]),
        ] {
            let plan = Plan::with_policy(root.clone(), region, budget(region), policy).unwrap();
            let recorded = Executor::default().prepare(&device, plan).unwrap();
            let destination = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("separate storage gather result"),
                size: u64::from(region.width) * u64::from(region.height) * 16,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            recorded
                .encode_into(&device, &mut encoder, &input, &destination)
                .unwrap();
            queue.submit([encoder.finish()]);
            let actual: Vec<_> = super::super::super::execute::read(&device, &queue, &destination)
                .into_iter()
                .map(|pixel| pixel.map(f32::to_bits))
                .collect();
            let expected: Vec<_> = (region.y..region.bottom())
                .flat_map(|y| (region.x..region.right()).map(move |x| bounds.index([x, y])))
                .map(|index| original[index])
                .collect();
            assert_eq!(
                actual, expected,
                "storage gathering must preserve NaN payloads, signed zero, subnormals and metadata"
            );
        }
    }
}

struct Literal(Vec<[f32; 4]>);
impl CpuKernel for Literal {
    fn run(
        &self,
        _: &[CpuImage],
        _: CellRegion,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<[f32; 4]>>, Error> {
        Ok((!cancelled()).then(|| self.0.clone()))
    }
}
fn argument(pixels: Vec<[f32; 4]>) -> Value {
    let region = CellRegion::new([0, 0], [pixels.len() as u32, 1]);
    Value::new(
        Domain {
            bounds: Some(region),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::Arguments,
        },
        region,
        vec![],
        Operation::cpu(
            "literal argument witness",
            pixels.clone(),
            Arc::new(Literal(pixels)),
        ),
    )
    .unwrap()
}

#[test]
fn concatenated_values_copy_distinct_inputs_without_shader_arithmetic_or_metadata_changes() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let gate = crate::gpu::submission::SubmissionGate::default();
    let a = vec![[1., -2., 3., -0.], [-5., 6., 7., 0.5]];
    let b = vec![
        [10_000., -40_000., 0.25, 1.],
        [17., 18., 19., 0.75],
        [2., 4., 8., 0.],
    ];
    let expected = a
        .iter()
        .chain(&b)
        .map(|pixel| pixel.map(f32::to_bits))
        .collect::<Vec<_>>();
    let value = Value::concatenate(vec![argument(a), argument(b)]).unwrap();
    let area = value.coverage();
    let plan = Plan::with_policy(value.clone(), area, budget(area), Policy::Composed).unwrap();
    assert!(
        plan.tiles
            .iter()
            .flat_map(|tile| &tile.fragment.work)
            .all(|work| !matches!(work, Work::Compute { .. })),
        "pure argument movement must not lower photographic or copy shaders"
    );
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let query = context
        .request(
            value.clone(),
            Region {
                bounds: area,
                grid: value.domain().grid,
            },
            budget(area),
        )
        .unwrap();
    let mut actual = Vec::new();
    context
        .read(
            &queue,
            &gate,
            &query,
            || false,
            |r, block| {
                assert_eq!(r, area);
                actual.extend(block.iter().map(|pixel| pixel.map(f32::to_bits)));
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        actual, expected,
        "copy offsets and byte payloads must preserve every argument channel"
    );
}

#[test]
fn cropped_image_values_flatten_requested_rows_without_changing_identity_or_metadata() {
    let full = CellRegion::new([-3, 5], [4, 3]);
    let pixels = (5..8)
        .flat_map(|y| (-3..1).map(move |x| [x as f32, y as f32, (x + y) as f32, -0.]))
        .collect::<Vec<_>>();
    let source = Value::new(
        Domain {
            bounds: Some(full),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::Neutral,
        },
        full,
        vec![],
        Operation::cpu(
            "strided image witness",
            pixels.clone(),
            Arc::new(Literal(pixels)),
        ),
    )
    .unwrap();
    let part = CellRegion::new([-2, 6], [2, 2]);
    let read = source.clone().read(part).unwrap();
    assert_eq!(read.identity(), source.identity());
    assert_eq!(read.coverage(), part);
    let value = Value::concatenate(vec![read]).unwrap();
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let query = context
        .request(
            value.clone(),
            Region {
                bounds: value.coverage(),
                grid: value.domain().grid,
            },
            budget(value.coverage()),
        )
        .unwrap();
    let mut actual = Vec::new();
    context
        .read(
            &queue,
            &Default::default(),
            &query,
            || false,
            |_, block| {
                actual.extend(block.iter().map(|pixel| pixel.map(f32::to_bits)));
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    let expected = [
        [-2., 6., 4., -0.],
        [-1., 6., 5., -0.],
        [-2., 7., 5., -0.],
        [-1., 7., 6., -0.],
    ]
    .map(|pixel: [f32; 4]| pixel.map(f32::to_bits));
    assert_eq!(
        actual, expected,
        "flattening a crop must use its real origin, row stride and exact bytes"
    );
}

#[test]
fn copy_admission_rejects_out_of_range_or_unaligned_transfers_before_execution() {
    for (source_offset, bytes) in [(4, 16), (0, 3)] {
        let domain = Plate::new().domain;
        let mut fragment = Fragment::default();
        let source = fragment.resource(Storage::Input {
            slot: 0,
            bytes: 16,
            writable: false,
        });
        let output = fragment.image(domain, CellRegion::new([0, 6], [1, 1]));
        fragment.work.push(Work::Copy {
            source,
            target: output.resource,
            source_offset,
            target_offset: 0,
            bytes,
        });
        let plan = Plan::from_work(fragment, output, 1024).unwrap();
        assert!(matches!(
            Executor::admit(&wgpu::Limits::default(), &plan),
            Err(Error::InvalidParameter(
                "value copy exceeds its declared storage"
            ))
        ));
    }
}

#[test]
fn window_values_preserve_cropped_domain_and_exact_pixels_without_copy_shaders() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let plate = Plate::new();
    let input = source(Arc::new(plate.clone()));
    let region = CellRegion::new([-2, 6], [9, 7]);
    let image = window(input, region).unwrap();
    let value = image.request(region).unwrap();
    assert_eq!(value.domain().bounds, Some(region));
    assert_eq!(value.identity(), image.identity());
    assert_eq!(plate.reads.load(Ordering::Relaxed), 0);
    let expected = (region.y..region.bottom())
        .flat_map(|y| (region.x..region.right()).map(move |x| [x, y]))
        .map(|cell| plate.pixel(cell).map(f32::to_bits))
        .collect::<Vec<_>>();
    for policy in [Policy::Materialized, Policy::Composed] {
        let plan = Plan::with_policy(image.clone(), region, budget(region), policy).unwrap();
        assert!(
            plan.tiles
                .iter()
                .flat_map(|tile| &tile.fragment.work)
                .any(|work| matches!(work, Work::Copy { .. })),
            "the standalone window control must exercise actual byte movement"
        );
        assert!(
            plan.tiles
                .iter()
                .flat_map(|tile| &tile.fragment.work)
                .all(|work| !matches!(work, Work::Compute { .. })),
            "a physical image restriction must not invent pixel mathematics"
        );
        let result = execute::Executor::default()
            .run(&device, &queue, &Default::default(), &plan, || false)
            .unwrap()
            .unwrap();
        assert_eq!(
            execute::read(&device, &queue, &result.pixels)
                .iter()
                .map(|pixel| pixel.map(f32::to_bits))
                .collect::<Vec<_>>(),
            expected
        );
    }
}
