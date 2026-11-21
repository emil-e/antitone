use super::*;
use crate::gpu::evaluation::numeric_fixture::NumericKey;
use std::sync::Mutex;

mod admission;
mod counts;
mod coverage;
mod packing;
mod planning;
mod resident;
mod values;

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
        Some(RetainedKey::new(NumericKey(self.values.clone())))
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
fn fixture(size: [u32; 2], values: Vec<[f32; 4]>) -> (Value, Arc<Mutex<Vec<Rect>>>) {
    let reads = Arc::new(Mutex::new(Vec::new()));
    (
        source(Arc::new(Pixels {
            domain: Domain {
                bounds: Some(crate::gpu::evaluation::CellRegion::new([0, 0], size)),
                grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            },
            values: Arc::new(values),
            reads: reads.clone(),
        })),
        reads,
    )
}
#[derive(Debug, PartialEq, Hash)]
struct Square;
impl Operation for Square {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(Square))
    }
    fn reads(&self, region: Rect) -> Result<Vec<crate::gpu::evaluation::Read>, Error> {
        Ok(vec![crate::gpu::evaluation::Read::new(0, region)])
    }
    fn calculation(
        self: std::sync::Arc<Self>,
        inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _budget: crate::gpu::evaluation::Tiling,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, crate::Error> {
        let inputs = crate::gpu::evaluation::value::Calculation::references(inputs);
        let function = Function {
            code:
                "fn square(p: vec4f, position: vec2f) -> vec4f { return vec4f(p.rgb*p.rgb,p.w); }"
                    .into(),
            symbol: "square",
            bindings: vec![],
        };
        value::Calculation::new(
            domain,
            output,
            vec![inputs[0].request(output)?],
            value::Operation::shader_function(self.key(), function),
        )
        .map(Some)
    }
}

// The same square with explicit natural units exercises producer assembly and sharing.
struct TiledSquare;
impl Operation for TiledSquare {
    fn key(&self) -> Option<RetainedKey> {
        Square.key()
    }
    fn work_units(&self, region: Rect, _: Tiling) -> Result<WorkUnits, Error> {
        tiled_units(region, [32, 32])
    }
    fn calculation(
        self: Arc<Self>,
        inputs: &[Value],
        domain: Domain,
        region: Rect,
        tiling: Tiling,
    ) -> Result<Option<value::Calculation>, Error> {
        Arc::new(Square).calculation(inputs, domain, region, tiling)
    }
}

struct GpuIdentity;
impl Operation for GpuIdentity {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("test GPU worker boundary"))
    }
    fn calculation(
        self: Arc<Self>,
        inputs: &[Value],
        domain: Domain,
        region: Rect,
        _: Tiling,
    ) -> Result<Option<value::Calculation>, Error> {
        value::Calculation::new(
            domain,
            region,
            vec![value::Calculation::input(inputs[0].request(region)?)],
            value::Operation::new(
                "test GPU worker boundary",
                (),
                "fn pixel(p:vec2i,at:vec2f,value:vec4f)->vec4f { return value; }",
                "pixel",
                value::Access::Pointwise,
            ),
        )
        .map(Some)
    }
}
struct CpuIdentity;
impl Operation for CpuIdentity {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("test CPU worker boundary"))
    }
    fn calculation(
        self: Arc<Self>,
        inputs: &[Value],
        domain: Domain,
        region: Rect,
        _: Tiling,
    ) -> Result<Option<value::Calculation>, Error> {
        value::Calculation::new(
            domain,
            region,
            vec![value::Calculation::input(inputs[0].request(region)?)],
            value::Operation::cpu("test CPU worker boundary", self.key(), self),
        )
        .map(Some)
    }
}
impl CpuKernel for CpuIdentity {
    fn run(
        &self,
        inputs: &[CpuImage],
        _: Rect,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<[f32; 4]>>, Error> {
        if cancelled() {
            return Ok(None);
        }
        Ok(Some(inputs[0].pixels.clone()))
    }
}
// GPU -> CPU -> GPU exercises real worker production and readback without changing samples.
fn cpu_boundary(input: Value) -> Value {
    let input = Value::from_operation(input.domain, [input], GpuIdentity);
    Value::from_operation(input.domain, [input], CpuIdentity)
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
    }
}
fn materialize(root: Value, tile: [u32; 2]) -> Vec<[f32; 4]> {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.test_materializations(false);
    let domain = root.domain;
    let image = pollster::block_on(
        context
            .materialize(
                &queue,
                &scope,
                context
                    .request(
                        root,
                        Region {
                            bounds: domain.finite().unwrap(),
                            grid: domain.grid,
                        },
                        Tiling { tile },
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
        !crate::gpu::evaluation::unit::can_inline(
            &coarse,
            coarse.domain.finite().unwrap(),
            Tiling { tile: [2, 1] },
            None
        )
        .unwrap(),
        "control: the inner aperture must actually require bounded production"
    );
    let squared = Value::from_operation(coarse.domain, Some(coarse), Square);
    let second = Mapping {
        start: [0.; 2],
        axes: [[1024., 0.], [0., 256.]],
    };
    let mean = reconstruct(squared, output(second, [0, 0, 1, 1].into())).unwrap();
    let squared_mean = Value::from_operation(mean.domain, Some(mean.clone()), Square);
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
        crate::gpu::evaluation::unit::can_inline(
            &inner,
            Rect::new([0, 0], [1, 1]),
            Tiling { tile: [64, 32] },
            None
        )
        .unwrap(),
        "control: the same aperture supports a callable with a locally fitting dependency"
    );
    let root = Value::from_operation(inner.domain, Some(inner), Square);
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
    let alias = Value::from_operation(inner.domain, Some(inner), Alias);
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
    crate::gpu::evaluation::unit::plan_with_limits(
        &alias,
        alias.domain.finite().unwrap(),
        budget,
        Some(&limits),
    )
    .unwrap()
    .admit_for(&limits)
    .unwrap();
    let root = Value::from_operation(alias.domain, Some(alias), Square);
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
                bounds: root.domain.finite().unwrap(),
                grid: root.domain.grid,
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
    let actual = super::super::super::execute::read(&device, &queue, image.as_buffer().unwrap());
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
    let squared = Value::from_operation(coarse.domain, Some(coarse), Square);
    let second = Mapping {
        start: [0.; 2],
        axes: [[257. * 257., 0.], [0., 257. * 257.]],
    };
    let root = reconstruct(squared, output(second, [0, 0, 1, 1].into())).unwrap();
    assert!(
        !crate::gpu::evaluation::unit::can_inline(
            &root,
            root.domain.finite().unwrap(),
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
fn nested_workers_execute_without_a_parent_residual_byte_allowance() {
    let (input, reads) = fixture([512, 256], vec![[1.; 4]; 512 * 256]);
    let map = Mapping {
        start: [0.; 2],
        axes: [[512., 0.], [0., 256.]],
    };
    let inner =
        reconstruct_mapped(cpu_boundary(input), output(map, [0, 0, 1, 1].into()), map).unwrap();
    let root = Value::from_operation(inner.domain, Some(inner), Square);
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.test_materializations(false);
    let request = context
        .request(
            root.clone(),
            Region {
                bounds: root.domain.finite().unwrap(),
                grid: root.domain.grid,
            },
            Tiling { tile: [1, 1] },
        )
        .unwrap();
    let plan = request.plans().next().unwrap().unwrap();
    assert!(
        plan.tiles
            .iter()
            .flat_map(|tile| &tile.fragment.work)
            .any(|work| matches!(work, Work::Produce { .. }))
    );
    assert!(
        reads.lock().unwrap().is_empty(),
        "planning remains metadata-only"
    );
    let result = pollster::block_on(
        context
            .materialize(
                &queue,
                &scope,
                request,
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    );
    let image = result.unwrap();
    assert_eq!(
        super::super::super::execute::read(&device, &queue, image.as_buffer().unwrap()),
        vec![[1., 1., 1., 9.]]
    );
    assert_eq!(
        reads
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.width * r.height)
            .sum::<u32>(),
        512 * 256
    );
    assert_eq!(context.retained_bytes(), 0);
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
    let root = Value::from_operation(inner.domain, Some(inner), Square);
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
                bounds: root.domain.finite().unwrap(),
                grid: root.domain.grid,
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
            domain: root.domain,
            region: root.domain.finite().unwrap(),
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
        super::super::super::execute::read(&device, &queue, &pixels),
        vec![[1234.; 4]]
    );
}

#[test]
fn shared_neighbourhoods_preserve_individual_results_without_reacquiring_the_same_source() {
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
    let budget = Tiling { tile: [1, 1] };
    let expected: Vec<_> = roots
        .iter()
        .map(|root| materialize(root.clone(), budget.tile))
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
        context.materialize_many(&queue, &scope, requests).unwrap(),
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
    assert_eq!(
        reads
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.width * r.height)
            .sum::<u32>(),
        128 * 64
    );
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
                    Tiling { tile: [1, 1] },
                )
                .unwrap()
        })
        .collect();
    let images = pollster::block_on(futures_util::future::try_join_all(
        context.materialize_many(&queue, &scope, requests).unwrap(),
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
    let input = Value::from_operation(input.domain, Some(input), TiledSquare);
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
                    Tiling { tile: [2, 2] },
                )
                .unwrap()
        })
        .collect();
    let futures = context
        .materialize_many(&queue, &scope, evaluations)
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
    let size = [1024, 128];
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
    let input = Value::from_operation(input.domain, Some(input), TiledSquare);
    let map = Mapping {
        start: [0.; 2],
        axes: [[1024., 0.], [0., 128.]],
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
            Tiling { tile: [1, 1] },
        )
        .unwrap();
    let unit = root
        .plan_unit(root.domain.finite().unwrap(), Tiling { tile: [1, 1] })
        .unwrap();
    let UnitWork::Gpu(gpu) = unit.work else {
        panic!("GPU reconstruction expected")
    };
    let contributions = gpu
        .phases
        .map(|phase| phase.unwrap())
        .filter(|phase| !phase.reads.is_empty())
        .count();
    assert!(
        contributions > 1,
        "the aperture naturally spans multiple source contributions"
    );

    for delivery in ["materialize", "read", "sink"] {
        reads.lock().unwrap().clear();
        let actual = match delivery {
            "materialize" => materialize(root.clone(), [1, 1]),
            "read" => {
                let mut actual = Vec::new();
                context
                    .read(
                        &queue,
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
        assert!(reads.iter().all(|r| r.width <= 512 && r.height <= 128));
        assert_eq!(
            reads
                .iter()
                .map(|r| u64::from(r.width) * u64::from(r.height))
                .sum::<u64>(),
            1024 * 128
        );
    }
    reads.lock().unwrap().clear();
    assert!(
        context
            .read(
                &queue,
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
    let size = [512, 512];
    let mut values = vec![[-2., 3., 5., 1.]; 512 * 512];
    for y in 0..32 {
        for x in 0..32 {
            for (x, y) in [(x, y), (511 - x, y), (x, 511 - y), (511 - x, 511 - y)] {
                values[y * 512 + x] = [9999.; 4];
                values[y * 512 + x][3] = 4.;
            }
        }
    }
    values[254 * 512] = [9999., 9999., 9999., 2.]; // touches only one diamond vertex
    let (input, reads) = fixture(size, values);
    let map = Mapping {
        start: [256., 0.],
        axes: [[256., 256.], [-256., 256.]],
    };
    let actual = materialize(
        reconstruct_mapped(input, output(map, [0, 0, 1, 1].into()), map).unwrap(),
        [1, 1],
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
    let mut values = vec![[0., -2., 4., 1.]; 512 * 256];
    values[128 * 512 + 256][0] = 131_072.;
    let (input, reads) = fixture([512, 256], values);
    let input = super::super::super::operators::gaussian(input, 1.).unwrap();
    let input = cpu_boundary(input);
    let map = Mapping {
        start: [0.; 2],
        axes: [[512., 0.], [0., 256.]],
    };
    let actual = materialize(
        reconstruct_mapped(input, output(map, [0, 0, 1, 1].into()), map).unwrap(),
        [1, 1],
    );
    // The isolated interior impulse keeps its mass under a unit-sum blur, across source units.
    for (a, b) in actual[0][..3].iter().zip([1., -2., 4.]) {
        assert!((*a - b).abs() < 2e-5, "{actual:?}");
    }
    assert_eq!(actual[0][3], 9.);
    let reads = reads.lock().unwrap();
    assert!(reads.len() > 1);
    assert!(
        reads.iter().all(
            |r| r.width <= phases::SOURCE_TILE[0] + 8 && r.height <= phases::SOURCE_TILE[1] + 8
        )
    );
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
    let direct = direct.unwrap();
    drop(direct);
    let unit = root
        .plan_unit(root.domain.finite().unwrap(), budget)
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
        submissions.load(Ordering::SeqCst) > 0,
        "the bounded reduction must actually submit GPU work"
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
fn invalid_addressing_rejects_before_io_and_valid_neighbourhoods_propagate_source_failure() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Unreadable(Arc<AtomicUsize>);
    impl Source for Unreadable {
        fn domain(&self) -> Domain {
            Domain {
                bounds: Some(Rect::new([0; 2], [100_000; 2])),
                grid: Grid::axis_aligned([0.; 2], [1.; 2]),
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
        (100., Error::Gpu("unexpected source IO".into())),
        (
            20_000.,
            Error::InvalidParameter("evaluation raster exceeds GPU cell addressing"),
        ),
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
                Tiling { tile: [16; 2] },
            )
            .unwrap();
        let result = context.read(
            &queue,
            &evaluation,
            || false,
            |_, _| panic!("inadmissible neighbourhood delivered"),
        );
        let correct = match (&result, &expected) {
            (Err(Error::Gpu(actual)), Error::Gpu(expected)) => actual == expected,
            (Err(Error::InvalidParameter(actual)), Error::InvalidParameter(expected)) => {
                actual == expected
            }
            _ => false,
        };
        assert!(correct, "{result:?}");
        assert_eq!(reads.swap(0, Ordering::Relaxed), usize::from(sigma == 100.));
    }
}

#[test]
fn a_normal_reduction_executes_without_a_per_request_byte_allowance() {
    let (input, reads) = fixture([128, 64], vec![[1., -2., 3., 8.]; 128 * 64]);
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
                &context.test_materializations(false),
                context
                    .request(
                        root,
                        Region {
                            bounds: domain.finite().unwrap(),
                            grid: domain.grid,
                        },
                        Tiling { tile: [1, 1] },
                    )
                    .unwrap(),
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    );
    let image = result.unwrap();
    assert_eq!(
        super::super::super::execute::read(&device, &queue, image.as_buffer().unwrap()),
        vec![[1., -2., 3., 8.]]
    );
    assert_eq!(*reads.lock().unwrap(), vec![Rect::new([0, 0], [128, 64])]);
}

#[test]
fn reflected_fractional_streaming_extends_real_edges_and_addresses_nonzero_tiled_outputs() {
    let size = [2048, 1040];
    let (input, reads) = fixture(
        size,
        (0..1040)
            .flat_map(|y| {
                (0..2048).map(move |x| {
                    [
                        x as f32 / 16. - 64.,
                        y as f32 / 16. - 32.,
                        (x * y) as f32 / 32768.,
                        1.,
                    ]
                })
            })
            .collect(),
    );
    let region: Rect = [7, 9, 3, 2].into();
    let map = Mapping {
        start: [(132.4 + 7. * 45.2) * 16., (-3.25 - 9. * 40.3) * 16.],
        axes: [[-45.2 * 16., 0.], [0., 40.3 * 16.]],
    };
    let root = reconstruct_mapped(input, output(map, region), map).unwrap();
    let whole = materialize(root.clone(), [3, 2]);
    let tiles = materialize(root, [1, 1]);
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
            let mx = mean(
                (132.4 - x as f64 * 45.2) * 16.,
                (132.4 - (x + 1) as f64 * 45.2) * 16.,
                2048.,
            ) / 16.;
            let my = mean(
                (-3.25 + y as f64 * 40.3) * 16.,
                (-3.25 + (y + 1) as f64 * 40.3) * 16.,
                1040.,
            ) / 16.;
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
