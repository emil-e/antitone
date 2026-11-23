use super::*;
use crate::gpu::evaluation::numeric_fixture::NumericKey;
use std::sync::atomic::{AtomicUsize, Ordering};

mod recording;

struct Pixels(Arc<AtomicUsize>);
impl Source for Pixels {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some([0, 0, 8, 4].into()),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        }
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("prepared coordinates"))
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok((region.y..region.bottom())
            .flat_map(|y| (region.x..region.right()).map(move |x| [x as f32, y as f32, -2., 8.]))
            .collect())
    }
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
            Tiling { tile: [8, 4] },
        )
        .unwrap()
        .without_retention()
}

#[test]
fn recording_capacity_includes_prerequisites_not_only_the_root_plan() {
    let (device, _, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let reads = Arc::new(AtomicUsize::new(0));
    let evaluation = request(&context, source(Arc::new(Pixels(reads.clone()))));
    let plan = || {
        let mut fragment = Fragment::default();
        let output = fragment.image(evaluation.job.image().domain(), evaluation.job.region());
        for _ in 0..=Plan::MAX_RECORDING_PASSES / 2 {
            fragment.compute(
                "@group(0) @binding(0) var<storage,read_write> output:array<vec4f>;\n                 @compute @workgroup_size(64) fn coordinates(@builtin(global_invocation_id) p:vec3u) {\n                 if p.x < 32u { output[p.x]=vec4f(f32(p.x%8u),f32(p.x/8u),-2.,8.); } }",
                "coordinates", vec![(0, Binding::Write(output.resource))], 1,
            );
        }
        Plan::from_work(fragment, output).unwrap()
    };
    let mut prepared = Preparation::new(evaluation.clone(), PreparedWork::Gpu(plan()));
    assert!(
        prepared.can_record(),
        "each program separately fits the recording capacity"
    );
    prepared.dependencies.push(Preparation::new(
        evaluation.clone(),
        PreparedWork::Gpu(plan()),
    ));
    assert!(
        !prepared.can_record(),
        "the caller's encoder would contain both programs"
    );
    assert!(matches!(
        prepared.record(&context),
        Err(Error::InvalidParameter(
            "image preparation requires production before recording"
        ))
    ));
    assert_eq!(reads.load(Ordering::SeqCst), 0);
}

#[test]
fn independent_preparations_join_the_same_pending_image() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let reads = Arc::new(AtomicUsize::new(0));
    let root = shifted(source(Arc::new(Pixels(reads.clone()))), 3.);
    let first = context
        .prepare_image(request(&context, root.clone()))
        .unwrap();
    let second = context.prepare_image(request(&context, root)).unwrap();
    let first = context
        .produce_images(&queue, vec![first])
        .unwrap()
        .remove(0);
    let second = context
        .produce_images(&queue, vec![second])
        .unwrap()
        .remove(0);
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    let (first, second) = pollster::block_on(futures_util::future::join(first, second));
    let (first, second) = (first.unwrap(), second.unwrap());
    assert!(Arc::ptr_eq(&first.image, &second.image));
    assert_eq!(reads.load(Ordering::SeqCst), 1);
    assert_eq!(context.retained_bytes(), 0);
}

struct UndeclaredReads;
impl Operation for UndeclaredReads {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("undeclared prepared reads"))
    }
    fn calculation(
        self: Arc<Self>,
        inputs: &[Value],
        domain: Domain,
        output: CellRegion,
        budget: Tiling,
    ) -> Result<Option<value::Calculation>, Error> {
        Arc::new(Sum).calculation(inputs, domain, output, budget)
    }
}

#[test]
fn incomplete_read_descriptions_use_ordinary_preparation() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let reads = Arc::new(AtomicUsize::new(0));
    let input = source(Arc::new(Pixels(reads.clone())));
    let root = Value::from_operation(input.domain(), [input.clone(), input], UndeclaredReads);
    let evaluation = request(&context, root.clone());
    assert!(context.prepare_dependencies(&evaluation).unwrap().is_none());
    let prepared = context.prepare_image(evaluation).unwrap();
    assert!(prepared.dependencies.is_empty());
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    let image = pollster::block_on(
        context
            .produce_images(&queue, vec![prepared])
            .unwrap()
            .remove(0),
    )
    .unwrap();
    let pixels = super::super::super::execute::read(&device, &queue, image.as_buffer().unwrap());
    assert_eq!(pixels[0], [0., 0., -4., 8.]);
    assert_eq!(pixels[31], [14., 6., -4., 8.]);
    assert_eq!(context.retained_bytes(), 0);
}

struct SeparatePixels {
    reads: Arc<AtomicUsize>,
    identity: usize,
}
impl Source for SeparatePixels {
    fn domain(&self) -> Domain {
        Pixels(self.reads.clone()).domain()
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new((
            "separate prepared pixels",
            self.identity,
        )))
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        Pixels(self.reads.clone()).read(region)
    }
}

struct Sum;
impl Operation for Sum {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("prepared sum"))
    }
    fn reads(&self, output: CellRegion) -> Result<Vec<Read>, Error> {
        Ok(vec![Read::new(0, output), Read::new(1, output)])
    }
    fn calculation(
        self: Arc<Self>,
        inputs: &[Value],
        domain: Domain,
        output: CellRegion,
        _: Tiling,
    ) -> Result<Option<value::Calculation>, Error> {
        let inputs = value::Calculation::references(inputs);
        let operation = value::Operation::new(
            "prepared sum",
            (),
            "fn pixel(p:vec2i,position:vec2f,a:vec4f,b:vec4f)->vec4f { return vec4f(a.rgb+b.rgb,a.w); }",
            "pixel",
            value::Access::Pointwise,
        );
        value::Calculation::new(
            domain,
            output,
            vec![inputs[0].request(output)?, inputs[1].request(output)?],
            operation,
        )
        .map(Some)
    }
}

#[test]
fn multiple_missing_operands_execute_without_completed_source_pins() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let reads = Arc::new(AtomicUsize::new(0));
    let inputs = [0, 1].map(|identity| {
        source(Arc::new(SeparatePixels {
            reads: reads.clone(),
            identity,
        }))
    });
    let root = Value::from_operation(inputs[0].domain(), inputs, Sum);
    let evaluation = request(&context, root);
    let prepared = context.prepare_image(evaluation).unwrap();
    assert!(matches!(prepared.work, PreparedWork::Produce(_)));
    assert!(prepared.dependencies.is_empty());
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    let scope = context.materializations();
    let working = RunWorkingSet::default();
    let worker = Worker {
        context: &context,
        queue: &queue,
        scope: &scope,
        working: &working,
        cancelled: &|| false,
        progress: &|| {},
    };
    let layout = ImageLayout {
        domain: prepared.evaluation.job.image().domain(),
        region: [0, 0, 8, 4].into(),
        format: ImageFormat::Rgba32Float,
        backing: ImageBacking::Buffer,
    };
    let image = prepared.produce(&worker, layout).unwrap();
    working.finish_gpu().unwrap();
    assert!(
        image.used_inputs().is_empty(),
        "completed results do not pin source operands"
    );
    let pixels = super::super::super::execute::read(&device, &queue, image.as_buffer().unwrap());
    assert_eq!(pixels[0], [0., 0., -4., 8.]);
    assert_eq!(pixels[31], [14., 6., -4., 8.]);
    assert_eq!(reads.load(Ordering::SeqCst), 2);
    assert_eq!(context.retained_bytes(), 0);
}

fn shifted(source: Value, delta: f32) -> Value {
    let operation = value::Operation::new(
        "prepared shift",
        NumericKey(delta),
        "struct Parameters { delta:vec4f } fn pixel(p:vec2i,position:vec2f,a:vec4f,v:Parameters)->vec4f { return vec4f(a.rgb+v.delta.xyz,a.w); }",
        "pixel",
        value::Access::Pointwise,
    )
    .with_uniform("Parameters", &[delta, 0., 0., 0.]);
    value::pointwise(source, operation).unwrap()
}

#[test]
fn prepared_requests_share_production_without_pinning_finished_source_inputs() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let reads = Arc::new(AtomicUsize::new(0));
    let input = source(Arc::new(Pixels(reads.clone())));
    let key = input
        .output([0, 0, 8, 4].into())
        .unwrap()
        .identity()
        .unwrap()
        .clone();
    let root = shifted(input.clone(), 3.);
    let mut requests = context
        .materialize_many(
            &queue,
            &context.materializations(),
            vec![request(&context, root.clone()), request(&context, root)],
        )
        .unwrap();
    let first = pollster::block_on(requests.remove(0)).unwrap();
    let second = pollster::block_on(requests.remove(0)).unwrap();
    assert!(Arc::ptr_eq(&first.image, &second.image));
    assert_eq!(reads.load(Ordering::SeqCst), 1);
    assert!(first.image.used_inputs().is_empty());
    assert!(
        context
            .materializations()
            .ready::<ImageLayout>(&key)
            .is_empty(),
        "completed outputs do not pin their finished source uploads"
    );
    drop(first);
    let changed = pollster::block_on(
        context
            .materialize_many(
                &queue,
                &context.materializations(),
                vec![request(&context, shifted(input, 5.))],
            )
            .unwrap()
            .remove(0),
    )
    .unwrap();
    assert_eq!(
        reads.load(Ordering::SeqCst),
        2,
        "zero optional retention does not promise reuse after the last source consumer"
    );
    let pixels = super::super::super::execute::read(&device, &queue, changed.as_buffer().unwrap());
    assert_eq!(pixels[0], [5., 0., -2., 8.]);
    assert_eq!(pixels[31], [12., 3., -2., 8.]);
    assert_eq!(context.retained_bytes(), 0);
    drop((second, changed));
    assert!(
        context
            .materializations()
            .ready::<ImageLayout>(&key)
            .is_empty()
    );
}
