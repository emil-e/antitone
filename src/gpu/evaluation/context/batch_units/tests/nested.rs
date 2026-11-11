use super::*;

struct Last {
    domain: Domain,
    output: CellRegion,
    bias: f32,
}
impl GpuKernel for Last {
    fn lower(
        &self,
        fragment: &mut Fragment,
        slots: &[Resource],
    ) -> Result<PlannedImage, crate::Error> {
        let bias = fragment.uniform(&[self.bias, 0., 0., 0.]);
        fragment.compute("@group(0) @binding(0) var<storage, read> input: array<vec4f>;
            @group(0) @binding(1) var<storage, read_write> state: array<u32>;
            @group(0) @binding(2) var<storage, read_write> output: array<vec4f>;
            @group(0) @binding(3) var<uniform> bias: vec4f;
            @compute @workgroup_size(1) fn last() { state[0] = 30u; output[0] = input[state[0]] + vec4f(bias.x, 0., 0., 0.); }",
            "last", vec![(0, Binding::Read(slots[0])), (1, Binding::Write(slots[1])), (2, Binding::Write(slots[2])), (3, bias)], 1);
        Ok(PlannedImage {
            domain: self.domain,
            region: self.output,
            resource: slots[2],
        })
    }
}

fn last_unit(input: Value, domain: Domain, region: CellRegion, bias: f32) -> WorkUnit {
    WorkUnit {
        inputs: vec![input],
        domain,
        output: region,
        work: UnitWork::Gpu(GpuUnit {
            state: vec![4],
            phases: Box::new(std::iter::once(Ok(GpuPhase {
                reads: vec![Read::new(0, [0, 0, 31, 1].into())],
                inputs: vec![UnitValue::Read(0), UnitValue::State(0), UnitValue::Output],
                kernel: Arc::new(Last {
                    domain,
                    output: region,
                    bias,
                }),
            }))),
        }),
    }
}

#[test]
fn aggregate_admission_of_a_nested_gpu_result_preserves_individually_admitted_roots() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let domain = Domain {
        bounds: Some([0, 0, 31, 1].into()),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Neutral,
    };
    let input = Value::new(
        domain,
        domain.finite().unwrap(),
        vec![],
        value::Operation::new(
            "generated coordinate field",
            (),
            "fn pixel(p:vec2i,at:vec2f)->vec4f { return vec4f(f32(p.x),-2.,3.,1.); }",
            "pixel",
            value::Access::Pointwise,
        ),
    )
    .unwrap();
    let region: CellRegion = [0, 0, 1, 1].into();
    let output_domain = Domain {
        bounds: Some(region),
        ..domain
    };
    let budget = Budget {
        tile: [1, 1],
        scratch_bytes: 1024,
    };
    let evaluations=[5f32,9.].map(|bias| {
        let root=Value::new(output_domain,region,vec![input.request(domain.finite().unwrap()).unwrap()],
            value::Operation::new("biased last coordinate",bias.to_bits(),
                "fn pixel(p:vec2i,at:vec2f,bias:vec4f)->vec4f { return input_0(vec2i(30,0))+vec4f(bias.x,0.,0.,0.); }",
                "pixel",value::Access::Neighborhood).with_uniform("vec4f",&[bias,0.,0.,0.])
        ).unwrap();
        context.request(root,Region {bounds:region,grid:domain.grid},budget).unwrap()
    });
    let gate = crate::gpu::submission::SubmissionGate::default();
    for (evaluation, bias) in evaluations.iter().zip([5., 9.]) {
        let scope = context.test_materializations(false);
        let working = HostWorkingSet::default();
        let worker = Worker {
            context: &context,
            queue: &queue,
            gate: &gate,
            scope: &scope,
            budget,
            working: &working,
            cancelled: &|| false,
            progress: &|| {},
        };
        let unit = last_unit(input.clone(), output_domain, region, bias)
            .admit_for(budget, &device.limits())
            .unwrap();
        let (image, _) = worker.compute_unit(evaluation.job.image(), unit).unwrap();
        assert_eq!(
            crate::gpu::evaluation::execute::read(&device, &queue, image.as_buffer().unwrap()),
            vec![[30. + bias, -2., 3., 1.]],
            "individual positive control"
        );
    }
    let scope = context.test_materializations(false);
    let working = HostWorkingSet::default();
    let worker = Worker {
        context: &context,
        queue: &queue,
        gate: &gate,
        scope: &scope,
        budget,
        working: &working,
        cancelled: &|| false,
        progress: &|| {},
    };
    let targets = evaluations
        .iter()
        .zip([5., 9.])
        .map(|(evaluation, bias)| Target {
            evaluation,
            layout: ImageLayout {
                domain: output_domain,
                region,
                format: ImageFormat::Rgba32Float,
                backing: ImageBacking::Buffer,
            },
            unit: Some(
                last_unit(input.clone(), output_domain, region, bias)
                    .admit_for(budget, &device.limits())
                    .unwrap(),
            ),
        })
        .collect();
    let images = worker
        .materialize_targets(targets, 40, 1024)
        .unwrap()
        .unwrap();
    for (image, value) in images.iter().zip([35., 39.]) {
        assert_eq!(
            crate::gpu::evaluation::execute::read(&device, &queue, image.as_buffer().unwrap()),
            vec![[value, -2., 3., 1.]]
        );
    }
}
