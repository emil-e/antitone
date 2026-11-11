use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use wgpu::util::DeviceExt;

#[derive(Clone)]
struct Pixels {
    named: bool,
    values: Vec<[f32; 4]>,
    reads: Arc<AtomicUsize>,
}
impl Pixels {
    fn new(named: bool) -> Self {
        Self {
            named,
            values: vec![[1., 0.5, -1., 1.], [2., 1., -2., 2.]],
            reads: Default::default(),
        }
    }
}
impl Source for Pixels {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some([7, 3, 2, 1].into()),
            grid: Grid::axis_aligned([0.25, -2.], [0.5, 1.25]),
            meaning: Meaning::Neutral,
        }
    }
    fn identity(&self) -> Option<RetainedKey> {
        self.named.then(|| {
            RetainedKey::new(
                self.values
                    .iter()
                    .map(|v| v.map(f32::to_bits))
                    .collect::<Vec<_>>(),
            )
        })
    }
    fn read(&self, r: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        assert!(self.domain().finite().unwrap().contains_region(r));
        self.reads.fetch_add(1, Ordering::Relaxed);
        Ok(self.values[(r.x - 7) as usize..(r.right() - 7) as usize].to_vec())
    }
}
fn square(input: Value) -> Value {
    pointwise(
        input,
        Meaning::Neutral,
        Function::new(
            "fn supplied_square(p:vec4f,position:vec2f)->vec4f {return vec4f(p.rgb*p.rgb,p.a);}",
            "supplied_square",
        ),
    )
}
fn request(context: &Context, root: Value) -> Evaluation {
    let domain = root.domain();
    context
        .request(
            root,
            Region {
                bounds: domain.finite().unwrap(),
                grid: domain.grid,
            },
            Budget {
                tile: [
                    domain.finite().unwrap().width,
                    domain.finite().unwrap().height,
                ],
                scratch_bytes: 1 << 20,
            },
        )
        .unwrap()
}
fn buffer(device: &wgpu::Device, values: &[[f32; 4]]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("explicit calculated input"),
        contents: bytemuck::cast_slice(values),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    })
}

#[test]
fn recorded_arguments_keep_the_complete_unnamed_calculation_and_own_values_without_retention() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.test_materializations(false);
    let data = Pixels::new(false);
    let first_square = square(source(Arc::new(data.clone())));
    assert!(
        first_square.identity().is_none(),
        "exercise an actual unnamed definition, not a memoized key"
    );
    assert!(
        !first_square.inputs.is_empty(),
        "supplying its value must not replace its upstream definition"
    );
    let input = SuppliedImage::from_buffer(
        first_square.clone(),
        first_square.domain().finite().unwrap(),
        buffer(&device, &[[1., 0.25, 1., 1.], [4., 1., 4., 2.]]).into(),
    )
    .unwrap();
    assert!(input.definition.same_definition(&first_square));
    assert!(!input.definition.inputs().is_empty());
    assert!(input.definition.source().is_none());
    let weak = Arc::downgrade(&input.image);
    let domain = first_square.domain();
    let root = reconstruct(
        square(first_square.clone()),
        Domain {
            bounds: Some([0, 0, 1, 1].into()),
            grid: Grid {
                coverage: 8,
                ..Grid::axis_aligned(domain.point([7., 3.]), [1., 1.25])
            },
            meaning: Meaning::Neutral,
        },
    )
    .unwrap();
    assert!(
        context
            .record(request(&context, root.clone()), scope.clone(), Vec::new())
            .is_err(),
        "control must need source acquisition without the supplied value"
    );
    let recorded = context
        .record(request(&context, root), scope.clone(), vec![input])
        .unwrap();
    scope.clear();
    drop(first_square);
    assert!(
        weak.upgrade().is_some(),
        "recorded consumers own required storage independently of optional retention"
    );
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 16,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    recorded
        .encode_result(&device, &mut encoder, &output)
        .unwrap();
    context
        .submit(&queue, &Default::default(), encoder, || false)
        .unwrap()
        .unwrap();
    let actual = execute::read(&device, &queue, &output);
    assert_eq!(
        actual,
        vec![[8.5, 0.53125, 8.5, 11.]],
        "square after supplied square, then mean: do not square after averaging or reapply the first operation"
    );
    assert_eq!(data.reads.load(Ordering::Relaxed), 0);
    assert_eq!(context.retained_bytes(), 0);
    drop(recorded);
    assert!(
        weak.upgrade().is_none(),
        "no hidden strong retention after the consumer leaves"
    );
}

#[test]
fn supplied_coverage_requires_the_actual_value_and_its_signed_storage_extent() {
    let (device, _, _) = crate::gpu::test_support::render_context();
    let data = Pixels::new(true);
    let domain = Domain {
        bounds: None,
        ..data.domain()
    };
    let calculation = || reconstruct(source(Arc::new(data.clone())), domain).unwrap();
    let definition = calculation();
    assert!(definition.identity().is_some());
    let region = CellRegion::new([-3, -2], [2, 1]);
    let supplied = SuppliedImage::from_buffer(
        definition.clone(),
        region,
        buffer(&device, &[[1., 0.5, -1., 1.]; 2]).into(),
    )
    .unwrap();
    assert!(supplied.covers(&definition, region));
    assert!(supplied.covers(calculation(), CellRegion::new([-2, -2], [1, 1])));
    for outside in [
        CellRegion::new([-2, -2], [2, 1]),
        CellRegion::new([-3, -3], [2, 1]),
        CellRegion::new([-4, -2], [3, 1]),
    ] {
        assert!(!supplied.covers(&definition, outside));
    }
    let mut other = data.clone();
    other.values[0][0] = 3.;
    let different_value = reconstruct(source(Arc::new(other)), domain).unwrap();
    assert!(!supplied.covers(&different_value, region));
    let different_grid = reconstruct(
        source(Arc::new(data.clone())),
        Domain {
            grid: Grid::axis_aligned([0.25, -2.], [1., 1.25]),
            ..domain
        },
    )
    .unwrap();
    assert!(!supplied.covers(&different_grid, region));
    let mut unnamed = data;
    unnamed.named = false;
    let unnamed = reconstruct(source(Arc::new(unnamed)), domain).unwrap();
    assert!(unnamed.identity().is_none());
    assert!(!supplied.covers(&unnamed, region));
}

#[test]
fn stored_values_match_reauthored_operations_above_the_same_opaque_argument() {
    let (device, _, _) = crate::gpu::test_support::render_context();
    let data = Pixels::new(false);
    let argument = source(Arc::new(data.clone()));
    let domain = Domain {
        bounds: None,
        ..argument.domain()
    };
    let definition = reconstruct(argument.clone(), domain).unwrap();
    let reauthored = reconstruct(argument, domain).unwrap();
    assert!(!definition.same_capture(&reauthored));
    assert!(definition.identity().is_none() && reauthored.identity().is_none());
    let region = data.domain().finite().unwrap();
    let supplied =
        SuppliedImage::from_buffer(definition, region, buffer(&device, &data.values).into())
            .unwrap();
    assert!(
        supplied.covers(&reauthored, region),
        "a new operation wrapper does not change its actual calculation"
    );
    let independent_argument = reconstruct(source(Arc::new(data)), domain).unwrap();
    assert!(
        !supplied.covers(&independent_argument, region),
        "equal fixture bytes do not identify an independent opaque argument"
    );
    assert!(
        !supplied.covers(square(reauthored), region),
        "an added anonymous calculation still needs its own value"
    );
}

#[test]
fn materialized_consumers_recycle_scratch_without_overwriting_a_supplied_value() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let data = Pixels::new(false);
    let definition = source(Arc::new(data.clone()));
    let pixels = buffer(&device, &data.values);
    let supplied = SuppliedImage::from_buffer(
        definition.clone(),
        definition.domain().finite().unwrap(),
        pixels.clone().into(),
    )
    .unwrap();
    let double = |input| {
        value::pointwise(input,Meaning::Neutral,value::Operation::new("double supplied pixels",(),"fn double(p:vec2i,at:vec2f,pixel:vec4f)->vec4f{return vec4f(pixel.rgb*2.0,pixel.a);}","double",value::Access::Pointwise)).unwrap()
    };
    let root = double(double(definition));
    let recorded = context
        .record(
            request(&context, root).with_policy(Policy::Materialized),
            context.test_materializations(false),
            vec![supplied],
        )
        .unwrap();
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 32,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    recorded
        .encode_result(&device, &mut encoder, &output)
        .unwrap();
    context
        .submit(&queue, &Default::default(), encoder, || false)
        .unwrap()
        .unwrap();
    assert_eq!(
        execute::read(&device, &queue, &output),
        vec![[4., 2., -4., 1.], [8., 4., -8., 2.]]
    );
    assert_eq!(execute::read(&device, &queue, &pixels), data.values);
    assert_eq!(data.reads.load(Ordering::Relaxed), 0);
}

#[test]
fn supplied_values_reject_another_definition_and_malformed_storage() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.test_materializations(false);
    let data = Pixels::new(true);
    let definition = source(Arc::new(data.clone()));
    let value = pollster::block_on(
        context
            .materialize(
                &queue,
                &Default::default(),
                &scope,
                request(&context, definition.clone()),
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap();
    let before = data.reads.load(Ordering::Relaxed);
    let mut other = data.clone();
    other.values[0][0] = 3.;
    let other = source(Arc::new(other));
    assert!(matches!(
        SuppliedImage::from_buffer(
            other.clone(),
            other.domain().finite().unwrap(),
            BufferImage::Produced(value.clone())
        ),
        Err(Error::InvalidParameter(
            "supplied storage belongs to a different image value"
        ))
    ));
    assert!(
        SuppliedImage::from_buffer(
            definition.clone(),
            definition.domain().finite().unwrap(),
            BufferImage::Produced(value)
        )
        .is_ok(),
        "the same value must remain usable with zero retention"
    );
    assert!(matches!(
        SuppliedImage::from_buffer(
            definition.clone(),
            definition.domain().finite().unwrap(),
            buffer(&device, &[[1.; 4]]).into()
        ),
        Err(Error::InvalidParameter(
            "materialized storage differs from its declared layout"
        ))
    ));
    let unreadable = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 32,
        usage: wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    assert!(matches!(
        SuppliedImage::from_buffer(
            definition.clone(),
            definition.domain().finite().unwrap(),
            unreadable.into()
        ),
        Err(Error::InvalidParameter(
            "materialized storage differs from its declared layout"
        ))
    ));
    assert_eq!(
        data.reads.load(Ordering::Relaxed),
        before,
        "argument validation must not acquire samples"
    );
}

#[test]
fn produced_photographic_values_can_supply_their_captured_group_and_resolve_stage_dependencies() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.materializations();
    let data = Pixels::new(true);
    let input = source(Arc::new(data.clone()));
    let make = |exposure| {
        let grading = crate::gpu::grading::Grading::new(
            &device.limits(),
            crate::processing::RollSettings {
                exposure,
                ..Default::default()
            },
            crate::adjustment_layers::AdjustmentLayerSnapshot::new(vec![]).unwrap(),
            data.domain().finite().unwrap().source().unwrap(),
        )
        .unwrap();
        let grade = grading.compose(input.clone()).unwrap();
        let root = window(gaussian(grade.clone(), 0.9).unwrap(), [8, 3, 1, 1].into()).unwrap();
        (grade, root)
    };
    let (grade, root) = make(1.0);
    let region = root.domain().finite().unwrap();
    assert_eq!(
        root.region_for(&grade, region).unwrap(),
        Some(data.domain().finite().unwrap())
    );
    assert_eq!(data.reads.load(Ordering::Relaxed), 0);
    let value = pollster::block_on(
        context
            .materialize(
                &queue,
                &Default::default(),
                &scope,
                request(&context, root.clone()),
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap();
    let before = data.reads.load(Ordering::Relaxed);
    assert!(before > 0);
    let supplied =
        SuppliedImage::from_buffer(root.clone(), region, BufferImage::Produced(value.clone()))
            .unwrap();
    assert!(supplied.covers(&root, region));
    let recorded = BufferImage::recorded(&root, region, buffer(&device, &[[0.; 4]])).unwrap();
    let BufferImage::Produced(recorded) = recorded else {
        panic!("named photographic output must stay identified")
    };
    assert_eq!(recorded.identity, value.identity);
    let (_, changed) = make(2.0);
    assert!(matches!(
        SuppliedImage::from_buffer(changed, region, BufferImage::Produced(value)),
        Err(Error::InvalidParameter(
            "supplied storage belongs to a different image value"
        ))
    ));
    assert_eq!(data.reads.load(Ordering::Relaxed), before);
}
