use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct FixedScratchKernel {
    value: f32,
    calls: Arc<AtomicUsize>,
    domain: Domain,
    output: CellRegion,
}
impl GpuKernel for FixedScratchKernel {
    fn lower(
        &self,
        fragment: &mut Fragment,
        slots: &[Resource],
    ) -> Result<PlannedImage, crate::Error> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let data = fragment.resource(Storage::Data(
            bytemuck::cast_slice(&[self.value; 248]).into(),
        ));
        fragment.compute(
            "@group(0) @binding(0) var<storage, read> data: array<f32>;
             @group(0) @binding(1) var<storage, read_write> output: array<vec4f>;
             @group(0) @binding(2) var<storage, read_write> state: array<f32>;
             @compute @workgroup_size(1) fn fixed() { state[0] = data[247]; output[0] = vec4f(state[0]); }",
            "fixed",
            vec![(0, Binding::Read(data)), (1, Binding::Write(slots[1])), (2, Binding::Write(slots[0]))],
            1,
        );
        Ok(PlannedImage {
            domain: self.domain,
            region: self.output,
            resource: slots[1],
        })
    }
}

#[test]
fn related_valid_roots_execute_once_without_fractional_working_reserves() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.test_materializations(false);
    let region: CellRegion = [0, 0, 1, 1].into();
    let domain = Domain {
        bounds: Some(region),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Neutral,
    };
    let budget = Tiling { tile: [1, 1] };
    let calls = [Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0))];
    let evaluations = [5f32, 9.].map(|value| {
        // Packed tables and persistent state must both participate in real execution.
        let root = Value::new(
            domain,
            region,
            vec![],
            value::Operation::new(
                "constant from captured table",
                value.to_bits(),
                "fn pixel(p:vec2i,at:vec2f)->vec4f { return bitcast<vec4f>(table_0(47u)); }",
                "pixel",
                value::Access::Pointwise,
            )
            .with_table(vec![[value.to_bits(); 4]; 48]),
        )
        .unwrap();
        let constrained = context
            .request(
                root.clone(),
                Region {
                    bounds: region,
                    grid: domain.grid,
                },
                budget,
            )
            .unwrap();
        let first = pollster::block_on(
            context
                .materialize(
                    &queue,
                    &context.test_materializations(false),
                    constrained,
                    ImageFormat::Rgba32Float,
                    ImageBacking::Buffer,
                )
                .unwrap(),
        );
        let first = first.unwrap();
        assert_eq!(
            crate::gpu::evaluation::execute::read(&device, &queue, first.as_buffer().unwrap()),
            vec![[value; 4]]
        );
        let evaluation = context
            .request(
                root,
                Region {
                    bounds: region,
                    grid: domain.grid,
                },
                budget,
            )
            .unwrap();
        let standalone = pollster::block_on(
            context
                .materialize(
                    &queue,
                    &context.test_materializations(false),
                    evaluation.clone(),
                    ImageFormat::Rgba32Float,
                    ImageBacking::Buffer,
                )
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            crate::gpu::evaluation::execute::read(&device, &queue, standalone.as_buffer().unwrap()),
            vec![[value; 4]]
        );
        evaluation
    });
    let targets = evaluations
        .iter()
        .zip([5., 9.])
        .zip(&calls)
        .map(|((evaluation, value), calls)| Target {
            evaluation,
            layout: ImageLayout {
                domain,
                region,
                format: ImageFormat::Rgba32Float,
                backing: ImageBacking::Buffer,
            },
            unit: Some(
                WorkUnit {
                    inputs: vec![],
                    domain,
                    output: region,
                    work: UnitWork::Gpu(GpuUnit {
                        state: vec![4],
                        phases: Box::new(std::iter::once(Ok(GpuPhase {
                            reads: vec![],
                            inputs: vec![UnitValue::State(0), UnitValue::Output],
                            kernel: Arc::new(FixedScratchKernel {
                                value,
                                calls: calls.clone(),
                                domain,
                                output: region,
                            }),
                        }))),
                    }),
                }
                .admit_for(&device.limits())
                .unwrap(),
            ),
        })
        .collect();
    let submissions = Arc::new(AtomicUsize::new(0));
    let observed = submissions.clone();
    let gate = queue.clone();
    gate.observe_submissions(Arc::new(move || {
        observed.fetch_add(1, Ordering::Relaxed);
    }));
    let working = RunWorkingSet::default();
    let worker = Worker {
        context: &context,
        queue: &queue,
        scope: &scope,
        working: &working,
        cancelled: &|| false,
        progress: &|| {},
    };
    let images = worker.materialize_targets(targets).unwrap().unwrap();
    for ((image, value), calls) in images.iter().zip([5., 9.]).zip(calls) {
        assert_eq!(
            crate::gpu::evaluation::execute::read(&device, &queue, image.as_buffer().unwrap()),
            vec![[value; 4]]
        );
        assert_eq!(
            calls.load(Ordering::Relaxed),
            1,
            "each backend executes once, without rejection and retry"
        );
    }
    assert_eq!(
        submissions.load(Ordering::Relaxed),
        2,
        "each valid root submits its declared phase"
    );
}
