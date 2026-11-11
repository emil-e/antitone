use super::*;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

mod batch;

#[derive(Default)]
struct Gate {
    entered: AtomicUsize,
    release: AtomicBool,
    cancelled: AtomicBool,
    cleanup: AtomicBool,
    returned: AtomicBool,
}
fn until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "execution boundary was not reached"
        );
        thread::sleep(Duration::from_millis(1));
    }
}
struct Ramp;
impl Source for Ramp {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some([0, 0, 8, 4].into()),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::Neutral,
        }
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("joined coordinate ramp"))
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, crate::Error> {
        Ok((region.y..region.bottom())
            .flat_map(|y| (region.x..region.right()).map(move |x| [x as f32, y as f32, 0., 0.]))
            .collect())
    }
}
struct GatedDouble {
    gate: Arc<Gate>,
    payload: Arc<Vec<u8>>,
}
impl CpuKernel for GatedDouble {
    fn run(
        &self,
        inputs: &[CpuImage],
        _: CellRegion,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<[f32; 4]>>, crate::Error> {
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].pixels.len(), 32);
        self.gate.entered.fetch_add(1, Ordering::AcqRel);
        while !self.gate.release.load(Ordering::Acquire) && !cancelled() {
            thread::sleep(Duration::from_millis(1));
        }
        if cancelled() {
            self.gate.cancelled.store(true, Ordering::Release);
            until(|| self.gate.cleanup.load(Ordering::Acquire));
            assert_eq!(
                self.payload[0], 0,
                "captured inputs survive physical cleanup"
            );
            self.gate.returned.store(true, Ordering::Release);
            return Ok(None);
        }
        let values = inputs[0]
            .pixels
            .iter()
            .map(|pixel| {
                [
                    2. * pixel[0] + f32::from(self.payload[0]),
                    2. * pixel[1],
                    0.,
                    pixel[3],
                ]
            })
            .collect();
        self.gate.returned.store(true, Ordering::Release);
        Ok(Some(values))
    }
}
fn root(gate: Arc<Gate>, payload: Arc<Vec<u8>>) -> Value {
    let source = source(Arc::new(Ramp));
    let domain = source.domain();
    let region = domain.finite().unwrap();
    let parameters = payload[0];
    value::Value::new(
        domain,
        region,
        vec![source.request(region).unwrap()],
        value::Operation::cpu(
            "double with captured offset",
            parameters,
            Arc::new(GatedDouble { gate, payload }),
        ),
    )
    .unwrap()
}
fn query(context: &Context, root: Value, region: CellRegion) -> Evaluation {
    context
        .request(
            root.clone(),
            Region {
                bounds: region,
                grid: root.domain().grid,
            },
            Budget {
                tile: [1, 1],
                scratch_bytes: 1 << 20,
            },
        )
        .unwrap()
}
fn consume(
    context: Context,
    queue: wgpu::Queue,
    evaluation: Evaluation,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<Result<Option<Vec<[f32; 4]>>, crate::Error>> {
    thread::spawn(move || {
        let mut pixels = Vec::new();
        let counts = context.read(
            &queue,
            &Default::default(),
            &evaluation,
            || stop.load(Ordering::Acquire),
            |_, values| {
                pixels.extend_from_slice(values);
                Ok(())
            },
        )?;
        Ok(counts.map(|_| pixels))
    })
}

#[test]
fn concurrent_regions_join_one_canonical_cpu_unit_with_independent_cancellation() {
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let gate = Arc::new(Gate::default());
    let root = root(gate.clone(), Arc::new(vec![0]));
    let first_stop = Arc::new(AtomicBool::new(false));
    let first = consume(
        context.clone(),
        queue.clone(),
        query(&context, root.clone(), [2, 1, 1, 1].into()),
        first_stop.clone(),
    );
    until(|| gate.entered.load(Ordering::Acquire) > 0);
    let second = consume(
        context.clone(),
        queue.clone(),
        query(&context, root.clone(), [3, 2, 1, 1].into()),
        Arc::new(AtomicBool::new(false)),
    );
    until(|| {
        context
            .materializations()
            .pending_consumers::<ImageLayout>(root.identity().unwrap(), |layout| {
                layout.backing == ImageBacking::Host
            })
            >= 2
    });
    assert_eq!(gate.entered.load(Ordering::Acquire), 1);
    first_stop.store(true, Ordering::Release);
    assert!(
        !gate.cancelled.load(Ordering::Acquire),
        "retiring one consumer must not cancel another's shared solve"
    );
    gate.release.store(true, Ordering::Release);
    assert!(first.join().unwrap().unwrap().is_none());
    assert_eq!(
        second.join().unwrap().unwrap().unwrap(),
        vec![[6., 4., 0., 0.]]
    );
    assert_eq!(gate.entered.load(Ordering::Acquire), 1);
    assert_eq!(context.retained_bytes(), 0);
}

#[test]
fn last_consumer_retirement_preserves_physical_inputs_until_cleanup_without_retaining_them_in_keys()
{
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let gate = Arc::new(Gate::default());
    let payload = Arc::new(vec![0]);
    let weak = Arc::downgrade(&payload);
    let root = root(gate.clone(), payload);
    let stop = Arc::new(AtomicBool::new(false));
    let consumer = consume(
        context.clone(),
        queue,
        query(&context, root, [2, 1, 1, 1].into()),
        stop.clone(),
    );
    until(|| gate.entered.load(Ordering::Acquire) > 0);
    stop.store(true, Ordering::Release);
    until(|| gate.cancelled.load(Ordering::Acquire));
    assert!(!consumer.is_finished());
    assert!(weak.upgrade().is_some());
    assert!(!gate.returned.load(Ordering::Acquire));
    gate.cleanup.store(true, Ordering::Release);
    assert!(consumer.join().unwrap().unwrap().is_none());
    until(|| weak.upgrade().is_none());
    assert!(gate.returned.load(Ordering::Acquire));
    assert_eq!(context.retained_bytes(), 0);
    assert!(context.materializations().is_empty());
}

struct ConstantCpu;
impl Operation for ConstantCpu {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("constant signed HDR CPU image"))
    }
    fn work_units(&self, _: CellRegion, _: Budget) -> Result<WorkUnits, crate::Error> {
        Ok(Box::new(std::iter::once([0, 0, 8, 4].into())))
    }
    fn calculation(
        self: Arc<Self>,
        _inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        _: CellRegion,
        _: Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, crate::Error> {
        Ok(Some(value::Calculation::new(
            domain,
            [0, 0, 8, 4].into(),
            Vec::new(),
            value::Operation::cpu("constant signed HDR CPU image", self.key(), self),
        )?))
    }
}
impl CpuKernel for ConstantCpu {
    fn run(
        &self,
        inputs: &[CpuImage],
        _: CellRegion,
        _: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<[f32; 4]>>, crate::Error> {
        assert!(inputs.is_empty());
        Ok(Some(vec![[2., -0.5, 0.25, 0.75]; 32]))
    }
}
#[test]
fn host_delivery_of_a_cpu_calculation_needs_no_gpu_upload_or_readback() {
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let root = Value::from_operation(Ramp.domain(), [], ConstantCpu);
    let query = query(&context, root, [2, 1, 2, 2].into());
    let mut values = Vec::new();
    let counts = context
        .read(
            &queue,
            &Default::default(),
            &query,
            || false,
            |_, part| {
                values.extend_from_slice(part);
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(values, vec![[2., -0.5, 0.25, 0.75]; 4]);
    assert_eq!(
        counts.submissions, 0,
        "host-to-host delivery must not make a GPU roundtrip"
    );
    assert_eq!(counts.buffer_allocations, 0);
}

fn copy_value(input: Value) -> Value {
    let domain = input.domain();
    let region = domain.finite().unwrap();
    Value::new(
        domain,
        region,
        vec![input.request(region).unwrap()],
        value::Operation::new(
            "identity pixel",
            (),
            "fn pixel(p:vec2i,at:vec2f,a:vec4f)->vec4f{return a;}",
            "pixel",
            value::Access::Pointwise,
        ),
    )
    .unwrap()
}
struct IdentityCpu;
impl CpuKernel for IdentityCpu {
    fn run(
        &self,
        inputs: &[CpuImage],
        _: CellRegion,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<[f32; 4]>>, Error> {
        Ok((!cancelled()).then(|| inputs[0].pixels.clone()))
    }
}
fn backend_value(input: Value) -> Value {
    let domain = input.domain();
    let region = domain.finite().unwrap();
    Value::new(
        domain,
        region,
        vec![input.request(region).unwrap()],
        value::Operation::cpu("backend identity value", (), Arc::new(IdentityCpu)),
    )
    .unwrap()
}
fn consume_backend(
    context: Context,
    queue: wgpu::Queue,
    evaluation: Evaluation,
    stop: Arc<AtomicBool>,
    unit: WorkUnit,
) -> thread::JoinHandle<Result<Option<Vec<[f32; 4]>>, crate::Error>> {
    thread::spawn(move || {
        let scope = context.materializations();
        let gate = crate::gpu::submission::SubmissionGate::default();
        let working = HostWorkingSet::default();
        let cancelled = || stop.load(Ordering::Acquire);
        let worker = Worker {
            context: &context,
            queue: &queue,
            gate: &gate,
            scope: &scope,
            working: &working,
            budget: evaluation.job.budget(),
            cancelled: &cancelled,
            progress: &|| {},
        };
        let whole = unit.output;
        let image =
            match worker.unit_image(evaluation.job.image(), unit, &mut WorkCounts::default()) {
                Err(Error::Cancelled) if cancelled() => return Ok(None),
                result => result?,
            };
        if cancelled() {
            return Ok(None);
        }
        let pixels = crate::gpu::evaluation::execute::read(
            context.device(),
            &queue,
            image.as_buffer().unwrap(),
        );
        let region = evaluation.region().bounds;
        Ok(Some(
            (region.y..region.bottom())
                .flat_map(|y| (region.x..region.right()).map(move |x| [x, y]))
                .map(|p| pixels[whole.index(p)])
                .collect(),
        ))
    })
}
struct WholeGpu {
    clears: Arc<AtomicUsize>,
}
struct ClearGpu {
    domain: Domain,
    region: CellRegion,
    clears: Arc<AtomicUsize>,
}
struct CopyGpu {
    domain: Domain,
    region: CellRegion,
}
impl GpuKernel for ClearGpu {
    fn lower(&self, f: &mut Fragment, slots: &[Resource]) -> Result<PlannedImage, crate::Error> {
        self.clears.fetch_add(1, Ordering::AcqRel);
        f.compute(
            "@group(0) @binding(0) var<storage,read_write> output:array<vec4f>;\n\
            @compute @workgroup_size(64) fn clear(@builtin(global_invocation_id) id:vec3u) {\
            if id.x<arrayLength(&output) { output[id.x]=vec4f(0.); }}",
            "clear",
            vec![(0, Binding::Write(slots[0]))],
            self.region.width * self.region.height,
        );
        Ok(PlannedImage {
            domain: self.domain,
            region: self.region,
            resource: slots[0],
        })
    }
}
impl GpuKernel for CopyGpu {
    fn lower(&self, f: &mut Fragment, slots: &[Resource]) -> Result<PlannedImage, crate::Error> {
        let parameters = f.uniform(&[[1f32, 0., 0., 0.]; 4]);
        f.compute(
            "@group(0) @binding(0) var<storage,read> input:array<vec4f>;\n\
            @group(0) @binding(1) var<storage,read_write> output:array<vec4f>;\n\
            @group(0) @binding(2) var<uniform> parameters:array<vec4f,4>;\n\
            @compute @workgroup_size(64) fn finish(@builtin(global_invocation_id) id:vec3u) {\
            if id.x<arrayLength(&output) { output[id.x]=vec4f(input[id.x].rgb*parameters[0].x,input[id.x].w); }}",
            "finish",
            vec![(0, Binding::Read(slots[0])), (1, Binding::Write(slots[1])), (2,parameters)],
            self.region.width * self.region.height,
        );
        Ok(PlannedImage {
            domain: self.domain,
            region: self.region,
            resource: slots[1],
        })
    }
}
impl WholeGpu {
    fn unit(&self, node: &Value) -> WorkUnit {
        let domain = node.domain();
        let output = domain.finite().unwrap();
        let phases = vec![
            GpuPhase {
                reads: vec![],
                inputs: vec![UnitValue::Output],
                kernel: Arc::new(ClearGpu {
                    domain,
                    region: output,
                    clears: self.clears.clone(),
                }),
            },
            GpuPhase {
                reads: vec![Read::new(0, output)],
                inputs: vec![UnitValue::Read(0), UnitValue::Output],
                kernel: Arc::new(CopyGpu {
                    domain,
                    region: output,
                }),
            },
        ];

        WorkUnit {
            inputs: node.inputs().to_vec(),
            domain,
            output,
            work: UnitWork::Gpu(GpuUnit {
                state: vec![],
                phases: Box::new(phases.into_iter().map(Ok)),
            }),
        }
    }
}
#[test]
fn retiring_a_queued_materialization_does_not_claim_or_run_its_unit() {
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let input = Value::from_operation(Ramp.domain(), [], ConstantCpu);
    let encoded = Arc::new(AtomicUsize::new(0));
    let observed = encoded.clone();
    let submissions = crate::gpu::submission::SubmissionGate::default();
    submissions.observe_encoded_resources(Arc::new(move || {
        observed.fetch_add(1, Ordering::Relaxed);
    }));
    let root = copy_value(input);
    let evaluation = context
        .request(
            root.clone(),
            Region {
                bounds: root.domain().finite().unwrap(),
                grid: root.domain().grid,
            },
            Budget {
                tile: [8, 4],
                scratch_bytes: 1 << 20,
            },
        )
        .unwrap();
    let scope = context.materializations();
    let pause = crate::background::testing::pause();
    let mut pending = Box::pin(
        context
            .materialize(
                &queue,
                &submissions,
                &scope,
                evaluation.clone(),
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    );
    assert!(
        futures_lite::future::block_on(futures_lite::future::poll_once(&mut pending)).is_none()
    );
    drop(pending);
    drop(pause);
    assert_eq!(encoded.load(Ordering::Acquire), 0);
    assert!(scope.is_empty());
    let ready = pollster::block_on(
        context
            .materialize(
                &queue,
                &submissions,
                &scope,
                evaluation,
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        read_buffer(&device, &queue, ready.as_buffer().unwrap()),
        vec![[2., -0.5, 0.25, 0.75]; 32]
    );
    assert!(
        encoded.load(Ordering::Acquire) > 0,
        "the new owner must actually encode GPU resources"
    );
}

#[test]
fn concurrent_regions_join_one_gpu_unit_while_its_upstream_cpu_dependency_is_running() {
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let gate = Arc::new(Gate::default());
    let input = root(gate.clone(), Arc::new(vec![0]));
    let clears = Arc::new(AtomicUsize::new(0));
    let root = backend_value(input);
    let first = consume_backend(
        context.clone(),
        queue.clone(),
        query(&context, root.clone(), [2, 1, 1, 1].into()),
        Arc::new(AtomicBool::new(false)),
        WholeGpu {
            clears: clears.clone(),
        }
        .unit(&root),
    );
    until(|| gate.entered.load(Ordering::Acquire) > 0);
    let second = consume_backend(
        context.clone(),
        queue,
        query(&context, root.clone(), [3, 2, 1, 1].into()),
        Arc::new(AtomicBool::new(false)),
        WholeGpu {
            clears: clears.clone(),
        }
        .unit(&root),
    );
    until(|| {
        context
            .materializations()
            .pending_consumers::<ImageLayout>(root.identity().unwrap(), |layout| {
                layout.backing == ImageBacking::Buffer
            })
            >= 2
    });
    assert_eq!(clears.load(Ordering::Acquire), 1);
    gate.release.store(true, Ordering::Release);
    assert_eq!(
        first.join().unwrap().unwrap().unwrap(),
        vec![[4., 2., 0., 0.]]
    );
    assert_eq!(
        second.join().unwrap().unwrap().unwrap(),
        vec![[6., 4., 0., 0.]]
    );
    assert_eq!(
        clears.load(Ordering::Acquire),
        1,
        "the GPU unit itself joins, not only its CPU dependency"
    );
    assert_eq!(context.retained_bytes(), 0);
}

struct BudgetSource(Arc<Gate>);
impl Source for BudgetSource {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some([0, 0, 1, 1].into()),
            ..Ramp.domain()
        }
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("gated constant budget source"))
    }
    fn read(&self, _: CellRegion) -> Result<Vec<[f32; 4]>, crate::Error> {
        self.0.entered.fetch_add(1, Ordering::AcqRel);
        until(|| self.0.release.load(Ordering::Acquire));
        Ok(vec![[3., 5., 7., 0.]])
    }
}
struct BudgetGpu;
impl BudgetGpu {
    fn unit(&self, node: &Value) -> WorkUnit {
        let domain = node.domain();
        let output = domain.finite().unwrap();
        let phases = vec![
            GpuPhase {
                reads: vec![],
                inputs: vec![UnitValue::Output],
                kernel: Arc::new(ClearGpu {
                    domain,
                    region: output,
                    clears: Arc::new(AtomicUsize::new(0)),
                }),
            },
            GpuPhase {
                reads: vec![Read::new(0, output)],
                inputs: vec![UnitValue::Read(0), UnitValue::Output],
                kernel: Arc::new(CopyGpu {
                    domain,
                    region: output,
                }),
            },
        ];

        WorkUnit {
            inputs: node.inputs().to_vec(),
            domain,
            output,
            work: UnitWork::Gpu(GpuUnit {
                state: vec![],
                phases: Box::new(phases.into_iter().map(Ok)),
            }),
        }
    }
}
#[test]
fn incompatible_pending_budgets_do_not_share_an_admission_failure_but_completed_values_are_reusable()
 {
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 1 << 20,
        },
    );
    let gate = Arc::new(Gate::default());
    let input = source(Arc::new(BudgetSource(gate.clone())));
    let root = backend_value(input);
    let small = context
        .request(
            root.clone(),
            Region {
                bounds: [0, 0, 1, 1].into(),
                grid: root.domain().grid,
            },
            Budget {
                tile: [1, 1],
                scratch_bytes: 64,
            },
        )
        .unwrap();
    let large = query(&context, root.clone(), [0, 0, 1, 1].into());
    let first = consume_backend(
        context.clone(),
        queue.clone(),
        small.clone(),
        Arc::new(AtomicBool::new(false)),
        BudgetGpu.unit(&root),
    );
    until(|| gate.entered.load(Ordering::Acquire) > 0);
    let second = consume_backend(
        context.clone(),
        queue.clone(),
        large,
        Arc::new(AtomicBool::new(false)),
        BudgetGpu.unit(&root),
    );
    until(|| gate.entered.load(Ordering::Acquire) >= 2);
    gate.release.store(true, Ordering::Release);
    assert_eq!(
        first.join().unwrap(),
        Err(crate::Error::WorkingAllowance(
            "evaluation tile exceeds scratch budget"
        ))
    );
    assert_eq!(
        second.join().unwrap().unwrap().unwrap(),
        vec![[3., 5., 7., 0.]]
    );
    assert_eq!(
        consume_backend(
            context,
            queue,
            small,
            Arc::new(AtomicBool::new(false)),
            BudgetGpu.unit(&root)
        )
        .join()
        .unwrap()
        .unwrap()
        .unwrap(),
        vec![[3., 5., 7., 0.]],
        "the small request can consume a completed image without producing it"
    );
    assert_eq!(gate.entered.load(Ordering::Acquire), 2);
}

struct PlusOne;
impl Operation for PlusOne {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("add one to RGB"))
    }
    fn reads(&self, region: CellRegion) -> Result<Vec<Read>, crate::Error> {
        Ok(vec![Read::new(0, region)])
    }
    fn calculation(
        self: std::sync::Arc<Self>,
        inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _budget: crate::gpu::evaluation::Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, crate::Error> {
        let inputs = crate::gpu::evaluation::value::Calculation::references(inputs);
        let function = Function::new(
            "fn plus_one(p:vec4f,at:vec2f)->vec4f {return vec4f(p.rgb+vec3f(1.),p.w);}",
            "plus_one",
        );
        value::Calculation::new(
            domain,
            output,
            vec![inputs[0].request(output)?],
            value::Operation::shader_function(self.key(), function),
        )
        .map(Some)
    }
}

#[test]
fn one_worker_pool_completes_nested_units_and_dependent_batches_in_both_orders() {
    const PROBE: &str = "ANTITONE_ONE_WORKER_UNIT_PROBE";
    if std::env::var_os(PROBE).is_some() {
        let (device, queue, _renderer) = crate::gpu::test_support::render_context();
        let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
        let input = Value::from_operation(Ramp.domain(), [], ConstantCpu);
        let root = copy_value(input);
        let evaluation = |node: Value| {
            context
                .request(
                    node.clone(),
                    Region {
                        bounds: node.domain().finite().unwrap(),
                        grid: node.domain().grid,
                    },
                    Budget {
                        tile: [8, 4],
                        scratch_bytes: 1 << 20,
                    },
                )
                .unwrap()
        };
        let result = pollster::block_on(
            context
                .materialize(
                    &queue,
                    &Default::default(),
                    &context.materializations(),
                    evaluation(root.clone()),
                    ImageFormat::Rgba32Float,
                    ImageBacking::Buffer,
                )
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            read_buffer(&device, &queue, result.as_buffer().unwrap()),
            vec![[2., -0.5, 0.25, 0.75]; 32]
        );
        let dependent = Value::from_operation(root.domain(), [root.clone()], PlusOne);
        for reverse in [false, true] {
            let scope = context.test_materializations(false);
            let nodes = if reverse {
                vec![dependent.clone(), root.clone()]
            } else {
                vec![root.clone(), dependent.clone()]
            };
            let results = pollster::block_on(futures_util::future::try_join_all(
                context
                    .materialize_many(
                        &queue,
                        &Default::default(),
                        &scope,
                        nodes.into_iter().map(evaluation).collect(),
                    )
                    .unwrap(),
            ))
            .unwrap();
            let expected = if reverse {
                [[3., 0.5, 1.25, 0.75], [2., -0.5, 0.25, 0.75]]
            } else {
                [[2., -0.5, 0.25, 0.75], [3., 0.5, 1.25, 0.75]]
            };
            for (image, expected) in results.iter().zip(expected) {
                assert_eq!(
                    read_buffer(&device, &queue, image.as_buffer().unwrap()),
                    vec![expected; 32]
                );
            }
        }
        return;
    }
    let mut child=std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact","gpu::evaluation::context::unit::tests::join::one_worker_pool_completes_nested_units_and_dependent_batches_in_both_orders","--nocapture"])
        .env(PROBE,"1").env("BLOCKING_MAX_THREADS","1")
        .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            let output = child.wait_with_output().unwrap();
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("running 1 test"),
                "the one-worker subprocess must discover exactly its intended case"
            );
            assert!(
                status.success(),
                "one-worker subprocess failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!(
                "one-worker subprocess deadlocked:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn malformed_host_backing_and_format_are_rejected_before_delivering_pixels() {
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    for format in [ImageFormat::Rgba32Float, ImageFormat::Rgba16Uint] {
        let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
        let root = Value::from_operation(Ramp.domain(), [], ConstantCpu);
        let layout = ImageLayout {
            domain: root.domain(),
            region: root.domain().finite().unwrap(),
            format,
            backing: ImageBacking::Host,
        };
        let image = if format == ImageFormat::Rgba32Float {
            ImageValue::new(device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: layout.bytes(),
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            }))
        } else {
            ImageValue::host(vec![[1.; 4]; 16])
        };
        let scope = context.materializations();
        let value = pollster::block_on(scope.obtain(
            root.identity().unwrap().clone(),
            Arc::new(layout),
            layout.bytes(),
            move || async move { Ok(image) },
        ))
        .unwrap();
        let mut delivered = false;
        let result = context.read(
            &queue,
            &Default::default(),
            &query(&context, root, [0, 0, 1, 1].into()),
            || false,
            |_, _| {
                delivered = true;
                Ok(())
            },
        );
        assert!(matches!(result, Err(crate::Error::InvalidParameter(_))));
        assert!(!delivered);
        drop(value);
    }
}

#[test]
fn a_host_image_is_not_mistaken_for_a_resident_gpu_buffer() {
    let (device, _queue, _renderer) = crate::gpu::test_support::render_context();
    let domain = Ramp.domain();
    let mut fragment = Fragment::default();
    let resource = fragment.resource(Storage::ResidentBuffer(ImageValue::host(vec![[1.; 4]; 32])));
    let result = PlannedImage {
        resource,
        domain,
        region: domain.finite().unwrap(),
    };
    let plan = Plan::from_work(fragment, result, 1 << 20).unwrap();
    assert!(matches!(
        Executor::admit(&device.limits(), &plan),
        Err(crate::Error::InvalidParameter(_))
    ));
}

#[test]
fn bounded_run_ownership_uses_common_discovery_without_optional_retention() {
    let (device, _queue, _renderer) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.materializations();
    let working = HostWorkingSet::new(32);
    let layout = ImageLayout {
        domain: Ramp.domain(),
        region: [0, 0, 2, 1].into(),
        format: ImageFormat::Rgba32Float,
        backing: ImageBacking::Host,
    };
    let first = RetainedKey::new(("constant host", 1u32));
    let second = RetainedKey::new(("constant host", 2u32));
    let publish = |identity: RetainedKey, value: f32| {
        let image = ImageValue::host(vec![[value; 4]; 2]);
        scope.submitted_image(&Image {
            identity,
            descriptor: Arc::new(layout),
            image: image.clone(),
        });
        working.hold(image);
    };
    publish(first.clone(), 1.);
    assert_eq!(scope.ready::<ImageLayout>(&first).len(), 1);
    publish(second.clone(), 2.);
    assert!(
        scope.ready::<ImageLayout>(&first).is_empty(),
        "the live working allowance must stay bounded"
    );
    assert_eq!(scope.ready::<ImageLayout>(&second).len(), 1);
    assert_eq!(context.retained_bytes(), 0);
    drop(working);
    assert!(
        scope.is_empty(),
        "the run owner, not the lookup, keeps active values alive"
    );
}
