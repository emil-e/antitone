use super::*;
mod admission;
mod nested;
use std::sync::Mutex;

struct Clear {
    domain: Domain,
    region: CellRegion,
}
impl GpuKernel for Clear {
    fn lower(&self, f: &mut Fragment, slots: &[Resource]) -> Result<PlannedImage, crate::Error> {
        f.compute(
            "@group(0) @binding(0) var<storage,read_write> total: array<vec4f>;\n\
            @compute @workgroup_size(64) fn clear(@builtin(global_invocation_id) id:vec3u) {\
            if id.x < arrayLength(&total) { total[id.x] = vec4f(0.0); }}",
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
struct Add {
    domain: Domain,
    region: CellRegion,
    weight: f32,
}
impl GpuKernel for Add {
    fn lower(&self, f: &mut Fragment, slots: &[Resource]) -> Result<PlannedImage, crate::Error> {
        let weight = f.uniform(&[self.weight, 0., 0., 0.]);
        f.compute(
            "@group(0) @binding(0) var<storage,read> input: array<vec4f>;\n\
            @group(0) @binding(1) var<storage,read_write> total: array<vec4f>;\n\
            @group(0) @binding(2) var<uniform> weight: vec4f;\n\
            @compute @workgroup_size(64) fn add(@builtin(global_invocation_id) id:vec3u) {\
            if id.x < arrayLength(&total) { total[id.x] += input[id.x] * weight.x; }}",
            "add",
            vec![
                (0, Binding::Read(slots[0])),
                (1, Binding::Write(slots[1])),
                (2, weight),
            ],
            self.region.width * self.region.height,
        );
        Ok(PlannedImage {
            domain: self.domain,
            region: self.region,
            resource: slots[1],
        })
    }
}
struct Finish {
    domain: Domain,
    region: CellRegion,
}
impl GpuKernel for Finish {
    fn lower(&self, f: &mut Fragment, slots: &[Resource]) -> Result<PlannedImage, crate::Error> {
        f.compute(
            "@group(0) @binding(0) var<storage,read> total: array<vec4f>;\n\
            @group(0) @binding(1) var<storage,read_write> output: array<vec4f>;\n\
            @compute @workgroup_size(64) fn finish(@builtin(global_invocation_id) id:vec3u) {\
            if id.x < arrayLength(&output) { output[id.x] = total[id.x]; }}",
            "finish",
            vec![(0, Binding::Read(slots[0])), (1, Binding::Write(slots[1]))],
            self.region.width * self.region.height,
        );
        Ok(PlannedImage {
            domain: self.domain,
            region: self.region,
            resource: slots[1],
        })
    }
}

struct Ramp(Arc<Mutex<Vec<CellRegion>>>);
impl Source for Ramp {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some([0, 0, 1000, 20].into()),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::Neutral,
        }
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        self.0.lock().unwrap().push(region);
        Ok((region.y..region.bottom())
            .flat_map(|y| (region.x..region.right()).map(move |x| [x as f32, y as f32, 0., 0.]))
            .collect())
    }
}

fn sum_unit(inputs: Vec<Value>, domain: Domain, region: CellRegion) -> WorkUnit {
    let phases = vec![
        GpuPhase {
            reads: vec![],
            inputs: vec![UnitValue::State(0)],
            kernel: Arc::new(Clear { domain, region }),
        },
        GpuPhase {
            reads: vec![Read::new(0, region)],
            inputs: vec![UnitValue::Read(0), UnitValue::State(0)],
            kernel: Arc::new(Add {
                domain,
                region,
                weight: 1.,
            }),
        },
        GpuPhase {
            reads: vec![Read::new(
                0,
                CellRegion::new([400, 10], [region.width, region.height]),
            )],
            inputs: vec![UnitValue::Read(0), UnitValue::State(0)],
            kernel: Arc::new(Add {
                domain,
                region,
                weight: 2.,
            }),
        },
        GpuPhase {
            reads: vec![],
            inputs: vec![UnitValue::State(0), UnitValue::Output],
            kernel: Arc::new(Finish { domain, region }),
        },
    ];

    WorkUnit {
        inputs,
        domain,
        output: region,
        work: UnitWork::Gpu(GpuUnit {
            state: vec![u64::from(region.width) * u64::from(region.height) * 16],
            phases: Box::new(phases.into_iter().map(Ok)),
        }),
    }
}

#[test]
fn declared_gpu_phases_keep_accumulator_state_across_disconnected_upstream_reads() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.test_materializations(false);
    let gate = crate::gpu::submission::SubmissionGate::default();
    let working = HostWorkingSet::default();
    let allowance = Budget {
        tile: [4, 1],
        scratch_bytes: 1 << 20,
    };
    let worker = Worker {
        context: &context,
        queue: &queue,
        gate: &gate,
        scope: &scope,
        budget: allowance,
        working: &working,
        cancelled: &|| false,
        progress: &|| {},
    };
    let region = CellRegion::new([0, 0], [4, 1]);
    let reads = Arc::new(Mutex::new(Vec::new()));
    let input = source(Arc::new(Ramp(reads.clone())));
    let domain = Domain {
        bounds: Some(region),
        ..input.domain()
    };
    let unit = sum_unit(vec![input], domain, region)
        .admit_for(allowance, &device.limits())
        .unwrap();
    assert!(reads.lock().unwrap().is_empty());
    let UnitWork::Gpu(work) = unit.work else {
        unreachable!()
    };
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("accumulator output"),
        size: 64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    worker.gpu_unit(unit.inputs, region, work, &output).unwrap();
    let pixels = crate::gpu::evaluation::execute::read(&device, &queue, &output);
    assert_eq!(
        pixels,
        vec![
            [800., 20., 0., 0.],
            [803., 20., 0., 0.],
            [806., 20., 0., 0.],
            [809., 20., 0., 0.]
        ]
    );
    let reads = reads.lock().unwrap();
    assert!(reads.iter().any(|r| r.contains_region(region)));
    assert!(
        reads
            .iter()
            .any(|r| r.contains_region([400, 10, 4, 1].into()))
    );
}
struct ObservedRamp {
    scope: Materializations,
    reads: Arc<Mutex<Vec<CellRegion>>>,
}
impl Source for ObservedRamp {
    fn domain(&self) -> Domain {
        Ramp(Arc::new(Mutex::new(Vec::new()))).domain()
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("round-owned batch ramp"))
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, crate::Error> {
        if region.x == 400 {
            assert!(
                self.scope
                    .ready::<ImageLayout>(&self.identity().unwrap())
                    .iter()
                    .all(|image| image.descriptor.region.x != 0),
                "the previous shared read must retire before the next source round, while this job is still running"
            );
        }
        self.reads.lock().unwrap().push(region);
        Ramp(Arc::new(Mutex::new(Vec::new()))).read(region)
    }
}

#[test]
fn related_declared_units_share_containing_reads_and_release_them_between_rounds() {
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    for generated_bytes in [0, 1 << 20] {
        let context = Context::new(&device, CachePolicy { generated_bytes });
        let scope = context.materializations();
        scope.set_test_pressure(crate::sys::MemoryPressure::Normal);
        let reads = Arc::new(Mutex::new(Vec::new()));
        let input = source(Arc::new(ObservedRamp {
            scope: scope.clone(),
            reads: reads.clone(),
        }));
        // Narrow first: sharing must follow read geometry, not the caller's root order.
        let evaluations = [2, 4]
            .map(|width| {
                let region = CellRegion::new([0, 0], [width, 1]);
                let domain = Domain { bounds: Some(region), ..input.domain() };
                let remote = CellRegion::new([400,10],[width,1]);
                let root = Value::new(domain, region,
                    vec![input.request(region).unwrap(), input.request(remote).unwrap()],
                    value::Operation::new("disconnected sum", (), "fn pixel(p:vec2i,at:vec2f)->vec4f { return input_0(p)+2.0*input_1(p+vec2i(400,10)); }", "pixel", value::Access::Neighborhood)
                ).unwrap();
                context
                    .request(
                        root.clone(),
                        Region {
                            bounds: region,
                            grid: root.domain().grid,
                        },
                        Budget {
                            tile: [4, 1],
                            scratch_bytes: 64 << 10,
                        },
                    )
                    .unwrap()
            })
            .to_vec();
        let allowance = Budget {
            tile: [4, 1],
            scratch_bytes: 64 << 10,
        };
        let working = HostWorkingSet::default();
        let gate = crate::gpu::submission::SubmissionGate::default();
        let worker = Worker {
            context: &context,
            queue: &queue,
            gate: &gate,
            scope: &scope,
            budget: allowance,
            working: &working,
            cancelled: &|| false,
            progress: &|| {},
        };
        let mut total = 0;
        let targets = evaluations
            .iter()
            .map(|evaluation| {
                let region = evaluation.region().bounds;
                let domain = evaluation.job.image().domain();
                let unit = sum_unit(vec![input.clone()], domain, region)
                    .admit_for(allowance, &device.limits())
                    .unwrap();
                total += u64::from(region.width) * u64::from(region.height) * 32;
                Target {
                    evaluation,
                    layout: ImageLayout {
                        domain,
                        region,
                        format: ImageFormat::Rgba32Float,
                        backing: ImageBacking::Buffer,
                    },
                    unit: Some(unit),
                }
            })
            .collect();
        let images = worker
            .materialize_targets(targets, total, allowance.scratch_bytes)
            .unwrap()
            .unwrap();
        for (image, width) in images.iter().zip([2, 4]) {
            let expected = (0..width)
                .map(|x| [800. + 3. * x as f32, 20., 0., 0.])
                .collect::<Vec<_>>();
            assert_eq!(
                crate::gpu::evaluation::execute::read(&device, &queue, image.as_buffer().unwrap()),
                expected
            );
        }
        assert_eq!(
            *reads.lock().unwrap(),
            vec![
                CellRegion::new([0, 0], [4, 1]),
                CellRegion::new([400, 10], [4, 1])
            ],
            "one source traversal per round, independently of optional retention"
        );
        assert!(
            scope
                .ready::<ImageLayout>(input.identity().unwrap())
                .is_empty(),
            "completed roots must not retain phase inputs"
        );
        if generated_bytes == 0 {
            assert_eq!(context.retained_bytes(), 0);
        } else {
            assert!(
                context.retained_bytes() > 0,
                "selected root outputs must actually use optional retention in the positive-cache arm"
            );
        }
    }
}
