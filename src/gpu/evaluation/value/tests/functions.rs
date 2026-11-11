use super::*;
use wgpu::util::DeviceExt;

fn expression(source: Value, region: CellRegion, work: u64, repeats: usize) -> Value {
    let pair = Value::new(source.domain(), region, vec![source.request(region).unwrap()], Operation::new(
        "record polynomial terms", (), "fn pixel(p:vec2i, position:vec2f, a:vec4f)->array<vec4f,2>{return array<vec4f,2>(vec4f(2.0*a.rgb,a.w),vec4f(a.rgb*a.rgb,0.0));}", "pixel", Access::Pointwise,
    ).with_shape(Shape::Pair).with_sampling_work(work)).unwrap();
    let mut half = pair.clone();
    for _ in 0..repeats {
        half = Value::new(pair.domain(), region, vec![half], Operation::new(
        "record half square", (), "fn pixel(p:vec2i, position:vec2f, a:array<vec4f,2>)->array<vec4f,2>{return array<vec4f,2>(a[0],a[1]*0.5);}", "pixel", Access::Pointwise,
    ).with_shape(Shape::Pair)).unwrap();
    }
    Value::new(pair.domain(), region, vec![pair,half], Operation::new(
        "record polynomial output", (), "fn pixel(p:vec2i, position:vec2f, a:array<vec4f,2>, b:array<vec4f,2>)->vec4f{return vec4f(a[0].rgb+b[1].rgb,a[0].w);}", "pixel", Access::Pointwise,
    )).unwrap()
}

#[test]
fn ordinary_private_records_share_math_between_functions_and_recorded_steps_without_source_io() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    for unbounded in [false, true] {
        let mut plate = Plate::new();
        let initial_boundary = plate.domain.bounds.unwrap();
        if unbounded {
            plate.domain.bounds = None;
        }
        let source = source(Arc::new(plate.clone()))
            .request(initial_boundary)
            .unwrap();
        let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
        for policy in [Policy::Materialized, Policy::Composed] {
            let mut hot_programs = None;
            for (region, repeats) in [
                (CellRegion::new([-4, 5], [4, 3]), 1),
                (CellRegion::new([1, 9], [3, 2]), 3),
            ] {
                let initial = CellRegion::new([region.x, region.y], [1, 1]);
                let root = expression(source.clone(), initial, 1, repeats)
                    .request(region)
                    .unwrap();
                assert!(
                    FunctionGraph::new(&Calculation::from_value(root.clone()).unwrap())
                        .unwrap()
                        .is_some()
                );
                let pixels = (region.y..region.bottom())
                    .flat_map(|y| (region.x..region.right()).map(move |x| [x, y]))
                    .map(|p| plate.pixel(p))
                    .collect::<Vec<_>>();
                let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("supplied polynomial input"),
                    contents: bytemuck::cast_slice(&pixels),
                    usage: wgpu::BufferUsages::STORAGE,
                });
                let supplied =
                    SuppliedImage::from_buffer(source.clone(), region, input.into()).unwrap();
                let query = context
                    .request(
                        root,
                        Region {
                            bounds: region,
                            grid: plate.domain.grid,
                        },
                        budget(region),
                    )
                    .unwrap();
                let query = query.with_policy(policy);
                let recorded = context
                    .record(query, context.materializations(), vec![supplied])
                    .unwrap();
                let programs = context.programs.len();
                if policy == Policy::Materialized
                    && let Some(hot) = hot_programs
                {
                    assert_eq!(
                        programs, hot,
                        "coverage and previous phase count must not produce new shader programs"
                    );
                }
                hot_programs = Some(programs);
                let output = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("recorded polynomial output"),
                    size: pixels.len() as u64 * 16,
                    usage: wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::COPY_DST
                        | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                });
                let mut encoder = device.create_command_encoder(&Default::default());
                recorded
                    .encode_result(&device, &mut encoder, &output)
                    .unwrap();
                queue.submit([encoder.finish()]);
                recorded.submitted();
                let actual = super::super::super::execute::read(&device, &queue, &output);
                assert_eq!(actual.len(), pixels.len());
                for (actual, source) in actual.iter().zip(&pixels) {
                    for channel in 0..3 {
                        assert_eq!(
                            actual[channel],
                            2.0 * source[channel]
                                + 0.5f32.powi(repeats as i32) * source[channel] * source[channel]
                        );
                    }
                    assert_eq!(actual[3].to_bits(), source[3].to_bits());
                }
            }
        }
        assert_eq!(plate.reads.load(Ordering::Relaxed), 0);
    }
}

#[test]
fn emitted_record_functions_execute_the_polynomial_on_supplied_pixels() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let plate = Plate::new();
    let source = source(Arc::new(plate.clone()))
        .request(plate.domain.finite().unwrap())
        .unwrap();
    let region = CellRegion::new([1, 9], [5, 3]);
    let repeats = 2;
    let initial = CellRegion::new([1, 9], [1, 1]);
    let definition = expression(source.clone(), initial, 1, repeats);
    let root = definition.request(region).unwrap();
    assert!(definition.same_capture(&root));
    let pixels = (region.y..region.bottom())
        .flat_map(|y| (region.x..region.right()).map(move |x| [x, y]))
        .map(|cell| plate.pixel(cell))
        .collect::<Vec<_>>();
    let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("direct record function input"),
        contents: bytemuck::cast_slice(&pixels),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let mut fragment = Fragment::default();
    let mut source_visits = 0;
    let sample = FunctionGraph::new(&Calculation::from_value(root.clone()).unwrap())
        .unwrap()
        .unwrap()
        .emit(&mut fragment, |value, demand, fragment| {
            assert!(value.same_capture(&source));
            assert_eq!(demand, region);
            source_visits += 1;
            Ok(Sampled::buffer(
                fragment,
                demand,
                Binding::Shared(Shared::Buffer(input.clone())),
            ))
        })
        .unwrap();
    assert_eq!(source_visits, 1);
    let output = fragment.image(root.domain(), region);
    sample.materialize(&mut fragment, output).unwrap();
    let plan = Plan::from_work(fragment, output, budget(region).scratch_bytes).unwrap();
    let result = Executor::default()
        .run(&device, &queue, &Default::default(), &plan, || false)
        .unwrap()
        .unwrap();
    let actual = super::super::super::execute::read(&device, &queue, &result.pixels);
    assert_eq!(actual.len(), pixels.len());
    for (actual, source) in actual.iter().zip(&pixels) {
        for channel in 0..3 {
            assert_eq!(
                actual[channel],
                2. * source[channel]
                    + 0.5f32.powi(repeats as i32) * source[channel] * source[channel]
            );
        }
        assert_eq!(actual[3].to_bits(), source[3].to_bits());
    }
    assert_eq!(plate.reads.load(Ordering::Relaxed), 0);
}

#[test]
fn recorded_neighbourhoods_materialize_callable_inputs_when_their_combined_bindings_do_not_fit() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let limit = device.limits().max_uniform_buffers_per_shader_stage;
    let region = CellRegion::new([0, 0], [16, 12]);
    let domain = Domain {
        bounds: Some(region),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Graded,
    };
    for uniforms in [limit - 3, limit - 1] {
        let declarations = (0..uniforms)
            .map(|slot| format!("@group(0) @binding({slot}) var<uniform> value_{slot}:vec4f;\n"))
            .collect::<String>();
        let sum = (0..uniforms)
            .map(|slot| format!("value_{slot}.x"))
            .collect::<Vec<_>>()
            .join("+");
        let code = format!(
            "{declarations} fn pixel(p:vec2i,position:vec2f)->vec4f{{let sum={sum};return vec4f(sum,-sum,sum*2.0,0.25);}}"
        );
        let mut captures = Function::new(code.clone(), "pixel");
        for slot in 0..uniforms {
            captures = captures.with_uniform(slot, &[slot as f32 + 1., 0., 0., 0.]);
        }
        let input = Value::new(
            domain,
            region,
            vec![],
            Operation::new(
                "GPU binding boundary witness",
                uniforms,
                code,
                "pixel",
                Access::Pointwise,
            )
            .with_bindings(captures.bindings),
        )
        .unwrap();
        let root = gaussian(input, 1.56).unwrap();
        let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
        let query = context
            .request(
                root,
                Region {
                    bounds: region,
                    grid: domain.grid,
                },
                budget(region),
            )
            .unwrap()
            .with_policy(Policy::Composed);
        let recorded = context
            .record(query, context.materializations(), vec![])
            .unwrap();
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bounded callable recording"),
            size: u64::from(region.width) * u64::from(region.height) * 16,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        recorded
            .encode_result(&device, &mut encoder, &output)
            .unwrap();
        queue.submit([encoder.finish()]);
        recorded.submitted();
        let actual = super::super::super::execute::read(&device, &queue, &output);
        let expected = (1..=uniforms).sum::<u32>() as f32;
        assert_eq!(
            actual,
            vec![
                [expected, -expected, 2. * expected, 0.25];
                (region.width * region.height) as usize
            ]
        );
    }
}

#[test]
fn callable_work_is_bounded_without_changing_mathematical_identity_or_compiling_shaders() {
    let plate = Plate::new();
    let source = source(Arc::new(plate.clone()))
        .request(plate.domain.finite().unwrap())
        .unwrap();
    let region = CellRegion::new([-4, 5], [4, 3]);
    let ordinary = expression(source.clone(), region, 1, 1);
    let expensive = expression(source, region, u64::MAX, 1);
    assert_eq!(ordinary.identity(), expensive.identity());
    assert!(
        FunctionGraph::new(&Calculation::from_value(ordinary.clone()).unwrap())
            .unwrap()
            .is_some()
    );
    assert!(
        FunctionGraph::new(&Calculation::from_value(expensive.clone()).unwrap())
            .unwrap()
            .is_none()
    );
    assert_eq!(plate.reads.load(Ordering::Relaxed), 0);
}
