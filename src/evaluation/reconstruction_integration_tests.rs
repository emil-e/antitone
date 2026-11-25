//! White-box checks of evaluator planning and delivery with affine reconstruction.

use super::*;
use crate::reconstruction::integration_tests::streaming::{
    Square, cpu_boundary, fixture, materialize, output,
};
use crate::reconstruction::{
    Mapping, SOURCE_TILE, reconstruct, reconstruct_complete, reconstruct_mapped,
};
use CellRegion as Rect;

mod admission;
mod planning;
mod resident;

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
        !crate::evaluation::unit::can_inline(
            &coarse,
            coarse.domain().finite().unwrap(),
            Tiling { tile: [2, 1] },
            None
        )
        .unwrap(),
        "control: the inner aperture must actually require bounded production"
    );
    let squared = Value::from_operation(coarse.domain(), Some(coarse), Square);
    let second = Mapping {
        start: [0.; 2],
        axes: [[1024., 0.], [0., 256.]],
    };
    let mean = reconstruct(squared, output(second, [0, 0, 1, 1].into())).unwrap();
    let squared_mean = Value::from_operation(mean.domain(), Some(mean.clone()), Square);
    for (root, expected) in [
        (mean, [2.5, 0.625, 2.5, 11.]),
        (squared_mean, [6.25, 0.390625, 6.25, 11.]),
    ] {
        let first_read = reads.lock().unwrap().len();
        let values = materialize(root, [1, 1]);
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
fn a_coalesced_nested_aperture_keeps_upstream_math_without_artificial_subdivision() {
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
        crate::evaluation::unit::can_inline(
            &inner,
            Rect::new([0, 0], [1, 1]),
            Tiling { tile: [64, 32] },
            None
        )
        .unwrap(),
        "control: the same aperture supports a callable with a locally fitting dependency"
    );
    let root = Value::from_operation(inner.domain(), Some(inner), Square);
    let actual = materialize(root, [64, 32]);
    for pixel in actual {
        for (a, b) in pixel.into_iter().zip([1., 4., 1., 8.]) {
            assert!((a - b).abs() < 3e-5, "{pixel:?} expected {b}");
        }
    }
    assert_eq!(
        *reads.lock().unwrap(),
        vec![Rect::new([0, 0], [1024, 512])],
        "the representable source window is acquired without artificial subdivision"
    );
}

#[test]
fn nested_reconstruction_uses_natural_contributions_when_coalescing_exceeds_a_binding() {
    struct Alias;
    impl Operation for Alias {
        fn key(&self) -> Option<RetainedKey> {
            Some(RetainedKey::new("test transparent reconstruction alias"))
        }
        fn calculation(
            self: Arc<Self>,
            inputs: &[Value],
            _domain: Domain,
            output: Rect,
            _tiling: Tiling,
        ) -> Result<Option<value::Calculation>, Error> {
            Ok(Some(value::Calculation::input(inputs[0].request(output)?)))
        }
    }
    let (input, reads) = fixture([512, 512], vec![[0.25, -2., 3., 0.]; 512 * 512]);
    let map = Mapping {
        start: [0.; 2],
        axes: [[16., 0.], [0., 16.]],
    };
    let inner = reconstruct_mapped(input, output(map, [0, 0, 32, 32].into()), map).unwrap();
    let alias = Value::from_operation(inner.domain(), Some(inner), Alias);
    let instance = crate::gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(crate::gpu::submission::request_device(
        &adapter,
        &wgpu::DeviceDescriptor {
            required_limits: wgpu::Limits {
                max_storage_buffer_binding_size: 1 << 20,
                ..Default::default()
            },
            ..Default::default()
        },
    ))
    .unwrap();
    let limits = device.limits();
    let budget = Tiling { tile: [32, 32] };
    crate::evaluation::unit::plan_with_limits(
        &alias,
        alias.domain().finite().unwrap(),
        budget,
        Some(&limits),
    )
    .unwrap()
    .admit_for(&limits)
    .unwrap();
    let root = Value::from_operation(alias.domain(), Some(alias), Square);
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.test_materializations(false);
    assert!(
        512 * 512 * 16 > device.limits().max_storage_buffer_binding_size,
        "control: the whole upstream input exceeds a storage binding"
    );
    let request = context
        .request(
            root.clone(),
            Region {
                bounds: root.domain().finite().unwrap(),
                grid: root.domain().grid,
            },
            budget,
        )
        .unwrap();
    let plan = request.plans().next().unwrap().unwrap();
    assert!(
        plan.tiles
            .iter()
            .flat_map(|tile| &tile.slots)
            .all(|storage| match storage {
                Storage::Buffer(bytes) => *bytes <= device.limits().max_storage_buffer_binding_size,
                Storage::Image { layout, .. } =>
                    layout.bytes() <= device.limits().max_storage_buffer_binding_size,
                _ => true,
            }),
        "the nested reconstruction stays within individual binding limits"
    );
    assert!(reads.lock().unwrap().is_empty());
    let image = pollster::block_on(
        context
            .materialize(
                &queue,
                &scope,
                request,
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap();
    let actual = crate::evaluation::read_buffer(&device, &queue, image.as_buffer().unwrap());
    assert_eq!(actual, vec![[0.0625, 4., 9., 8.]; 32 * 32]);
    let reads = reads.lock().unwrap();
    assert!(
        reads.len() > 1,
        "execution uses natural contributions, not an oversized whole binding"
    );
    assert!(reads.iter().all(
        |region| u64::from(region.width) * u64::from(region.height) * 16
            <= device.limits().max_storage_buffer_binding_size
    ));
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
    let squared = Value::from_operation(coarse.domain(), Some(coarse), Square);
    let second = Mapping {
        start: [0.; 2],
        axes: [[257. * 257., 0.], [0., 257. * 257.]],
    };
    let root = reconstruct(squared, output(second, [0, 0, 1, 1].into())).unwrap();
    assert!(
        !crate::evaluation::unit::can_inline(
            &root,
            root.domain().finite().unwrap(),
            Tiling { tile: [1, 1] },
            None
        )
        .unwrap()
    );
    let actual = materialize(root, [1, 1]);
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
fn cancellation_inside_a_nested_worker_does_not_publish_parent_pixels_or_read_more_originals() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use wgpu::util::DeviceExt;
    let (input, reads) = fixture([512, 384], vec![[1.; 4]; 512 * 384]);
    let map = Mapping {
        start: [0.; 2],
        axes: [[512., 0.], [0., 384.]],
    };
    let inner =
        reconstruct_mapped(cpu_boundary(input), output(map, [0, 0, 1, 1].into()), map).unwrap();
    let root = Value::from_operation(inner.domain(), Some(inner), Square);
    let (device, queue, _) = crate::photographic::test_support::render_context();
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
    let gate = queue.clone();
    let (stop, count) = (stopped.clone(), submissions.clone());
    gate.observe_submissions(Arc::new(move || {
        if count.fetch_add(1, Ordering::SeqCst) == 1 {
            stop.store(true, Ordering::SeqCst);
        }
    }));
    let budget = Tiling { tile: [1, 1] };
    let evaluation = context
        .request(
            root.clone(),
            Region {
                bounds: root.domain().finite().unwrap(),
                grid: root.domain().grid,
            },
            budget,
        )
        .unwrap()
        .with_materializations(scope.clone());
    let result = Production {
        worker: Worker {
            context: &context,
            queue: &queue,
            scope: &scope,
            working: &RunWorkingSet::default(),
            cancelled: &|| stopped.load(Ordering::SeqCst),
            progress: &|| {},
        },
        layout: ImageLayout {
            domain: root.domain(),
            region: root.domain().finite().unwrap(),
            format: ImageFormat::Rgba32Float,
            backing: ImageBacking::Buffer,
        },
        pixels: &pixels,
    }
    .produce(&evaluation);
    assert!(matches!(result, Err(Error::Cancelled)));
    assert_eq!(submissions.load(Ordering::SeqCst), 2);
    let acquired = reads.lock().unwrap();
    assert_eq!(acquired.len(), 2);
    assert_eq!(
        acquired.iter().map(|r| r.width * r.height).sum::<u32>(),
        512 * 256,
        "cancellation stops before the third input block"
    );
    assert_eq!(
        crate::evaluation::read_buffer(&device, &queue, &pixels),
        vec![[1234.; 4]]
    );
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
    let (device, queue, _) = crate::photographic::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.test_materializations(false);
    let budget = Tiling { tile: [512, 256] };
    let gate = queue.clone();
    let resident = pollster::block_on(
        context
            .materialize(
                &queue,
                &scope,
                context
                    .request(
                        input.clone(),
                        Region {
                            bounds: input.domain().finite().unwrap(),
                            grid: input.domain().grid,
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
                    bounds: root.domain().finite().unwrap(),
                    grid: root.domain().grid,
                },
                budget,
            )
            .unwrap(),
        scope.clone(),
        Vec::new(),
    );
    let direct = direct.unwrap();
    drop(direct);
    let unit = root
        .plan_unit(root.domain().finite().unwrap(), budget)
        .unwrap()
        .admit_for(&device.limits())
        .unwrap();
    let UnitWork::Gpu(work) = unit.work else {
        panic!("GPU reconstruction");
    };
    let phases = work.phases.collect::<Result<Vec<_>, _>>().unwrap();
    assert!(
        phases.len() > 1,
        "a large aperture must remain bounded steps, not one sampled loop"
    );
    assert!(
        phases
            .iter()
            .flat_map(|phase| &phase.reads)
            .all(|read| u64::from(read.region.width) * u64::from(read.region.height) <= 512 * 128)
    );
    assert_eq!(reads.lock().unwrap().len(), before);
    let image = pollster::block_on(
        context
            .materialize(
                &queue,
                &scope,
                context
                    .request(
                        root.clone(),
                        Region {
                            bounds: root.domain().finite().unwrap(),
                            grid: root.domain().grid,
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
        submissions.load(Ordering::SeqCst) > 0,
        "the bounded reduction must actually submit GPU work"
    );
    let actual = crate::evaluation::read_buffer(&device, &queue, image.as_buffer().unwrap());
    for (a, b) in actual[0][..3].iter().zip([1., -2., 4.]) {
        assert!((*a - b).abs() < 2e-5, "{actual:?}");
    }
    assert_eq!(actual[0][3], 9.);
    assert_eq!(context.retained_bytes(), 0);
    drop(resident);
}
