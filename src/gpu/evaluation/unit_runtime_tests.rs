use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

mod cpu_reads;

struct Alias;
impl Operation for Alias {
    fn describe(
        &self,
        output: Output,
        inputs: &[value::Value],
        _requested: &[CellRegion],
    ) -> Result<Output, Error> {
        inputs[0].output(output.region)
    }
    fn calculation(
        self: Arc<Self>,
        inputs: &[value::Value],
        _: Domain,
        output: CellRegion,
        _: Budget,
    ) -> Result<Option<value::Calculation>, Error> {
        Ok(Some(value::Calculation::input(inputs[0].request(output)?)))
    }
}

#[test]
fn cold_aliases_compile_owned_backend_units_and_reuse_their_actual_result() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let input = source(Arc::new(Ramp));
    let upstream = Value::from_operation(input.domain(), [input], Triple);
    let region = CellRegion::new([0, 0], [8, 4]);
    let calls = Arc::new(AtomicUsize::new(0));
    let core = Value::from_operation(
        Domain {
            bounds: Some(region),
            ..upstream.domain()
        },
        [upstream],
        WholeCpu {
            region,
            calls: calls.clone(),
            stop: None,
        },
    );
    let alias = Value::from_operation(core.domain(), [core.clone()], Alias);
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.materializations();
    let acquire = |root: Value| {
        let query = context
            .request(
                root,
                Region {
                    bounds: region,
                    grid: core.domain().grid,
                },
                Budget {
                    tile: [8, 4],
                    scratch_bytes: 1 << 20,
                },
            )
            .unwrap();
        pollster::block_on(
            context
                .materialize(
                    &queue,
                    &Default::default(),
                    &scope,
                    query,
                    ImageFormat::Rgba32Float,
                    ImageBacking::Buffer,
                )
                .unwrap(),
        )
        .unwrap()
    };
    let held = acquire(alias);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    let actual = read_buffer(&device, &queue, held.as_buffer().unwrap());
    assert_eq!(actual.len(), 32);
    for (index, pixel) in actual.iter().enumerate() {
        let x = (index % 8) as f32;
        let y = (index / 8) as f32;
        assert_eq!(*pixel, [9. * x + 2400., 9. * y + 60., 0., 0.]);
    }
    let reused = acquire(core.clone());
    assert!(Arc::ptr_eq(&held.image, &reused.image));
    assert_eq!(
        calls.load(Ordering::Relaxed),
        1,
        "the alias cannot cause another solve or wait on itself"
    );
}

#[test]
fn unsupported_definitions_reject_without_recursively_describing_themselves() {
    struct Undefined;
    impl Operation for Undefined {}
    let region = CellRegion::new([0, 0], [1, 1]);
    let root = Value::from_operation(
        Domain {
            bounds: Some(region),
            ..Ramp.domain()
        },
        [],
        Undefined,
    );
    assert!(matches!(
        super::unit::plan(
            &root,
            region,
            Budget {
                tile: [1, 1],
                scratch_bytes: 1024
            }
        ),
        Err(Error::InvalidParameter(
            "operation requires a declared executable calculation"
        ))
    ));
}

struct Ramp;
impl Source for Ramp {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some([0, 0, 1000, 20].into()),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::Neutral,
        }
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("coordinate ramp"))
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, crate::Error> {
        Ok((region.y..region.bottom())
            .flat_map(|y| (region.x..region.right()).map(move |x| [x as f32, y as f32, 0., 0.]))
            .collect())
    }
}

struct Triple;
impl Operation for Triple {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("triple RGB"))
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
            "fn triple(p: vec4f, at: vec2f) -> vec4f { return vec4f(3. * p.rgb, p.w); }",
            "triple",
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

struct WholeCpu {
    region: CellRegion,
    calls: Arc<AtomicUsize>,
    stop: Option<Arc<AtomicBool>>,
}
impl Operation for WholeCpu {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(self.region))
    }
    fn work_units(&self, _: CellRegion, _: Budget) -> Result<WorkUnits, crate::Error> {
        Ok(Box::new(std::iter::once(self.region)))
    }
    fn calculation(
        self: Arc<Self>,
        inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        _: CellRegion,
        _: Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, crate::Error> {
        let output = self.region;
        let donor = CellRegion::new(
            [output.x + 400, output.y + 10],
            [output.width, output.height],
        );
        Ok(Some(value::Calculation::new(
            domain,
            output,
            vec![
                value::Calculation::input(inputs[0].request(output)?),
                value::Calculation::input(inputs[0].request(donor)?),
            ],
            value::Operation::cpu("whole nonlocal CPU sum", self.key(), self),
        )?))
    }
}
impl CpuKernel for WholeCpu {
    fn run(
        &self,
        inputs: &[CpuImage],
        output: CellRegion,
        _: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<[f32; 4]>>, crate::Error> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        assert_eq!(output, self.region);
        assert_eq!(inputs.len(), 2);
        assert_eq!(inputs[0].region, self.region);
        assert_eq!(inputs[1].region.x, self.region.x + 400);
        assert!(
            inputs.iter().all(|input| input.pixels.len() == 32),
            "the CPU algorithm receives complete logical arrays, not upstream tiles"
        );
        if let Some(stop) = &self.stop {
            stop.store(true, Ordering::Release);
            return Ok(None);
        }
        Ok(Some(
            inputs[0]
                .pixels
                .iter()
                .zip(&inputs[1].pixels)
                .map(|(a, b)| std::array::from_fn(|channel| a[channel] + 2. * b[channel]))
                .collect(),
        ))
    }
}

fn cpu_graph(calls: Arc<AtomicUsize>, stop: Option<Arc<AtomicBool>>) -> Value {
    let input = source(Arc::new(Ramp));
    let upstream = Value::from_operation(input.domain(), [input], Triple);
    let region = CellRegion::new([0, 0], [8, 4]);
    let node = Value::from_operation(
        Domain {
            bounds: Some(region),
            ..upstream.domain()
        },
        [upstream],
        WholeCpu {
            region,
            calls,
            stop,
        },
    );
    pointwise(
        node,
        Meaning::Neutral,
        Function::new(
            "fn finish(p: vec4f, at: vec2f) -> vec4f { return p + vec4f(1., 2., 3., 0.); }",
            "finish",
        ),
    )
}

#[test]
fn gpu_cpu_gpu_chain_assembles_nonlocal_inputs_and_reuses_the_natural_unit() {
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 1 << 20,
        },
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let region = CellRegion::new([2, 1], [2, 2]);
    let root = cpu_graph(calls.clone(), None);
    let query = context
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
        .unwrap();
    assert_eq!(
        calls.load(Ordering::Relaxed),
        0,
        "graph construction must not run CPU work"
    );
    let mut pixels = Vec::new();
    context
        .read(
            &queue,
            &Default::default(),
            &query,
            || false,
            |_, block| {
                pixels.extend_from_slice(block);
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        pixels,
        vec![
            [2419., 71., 3., 0.],
            [2428., 71., 3., 0.],
            [2419., 80., 3., 0.],
            [2428., 80., 3., 0.]
        ]
    );
    assert_eq!(
        calls.load(Ordering::Relaxed),
        1,
        "four output requests share the same completed natural calculation unit"
    );
}

#[test]
fn a_whole_cpu_unit_larger_than_gpu_scratch_is_reused_across_output_tiles() {
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let calls = Arc::new(AtomicUsize::new(0));
    let root = cpu_graph(calls.clone(), None);
    let region = CellRegion::new([2, 1], [2, 2]);
    let query = context
        .request(
            root.clone(),
            Region {
                bounds: region,
                grid: root.domain().grid,
            },
            Budget {
                tile: [1, 1],
                scratch_bytes: 512,
            },
        )
        .unwrap();
    let mut pixels = Vec::new();
    context
        .read(
            &queue,
            &Default::default(),
            &query,
            || false,
            |_, block| {
                pixels.extend_from_slice(block);
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        pixels,
        vec![
            [2419., 71., 3., 0.],
            [2428., 71., 3., 0.],
            [2419., 80., 3., 0.],
            [2428., 80., 3., 0.]
        ]
    );
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[test]
fn cpu_cancellation_does_not_publish_a_partial_downstream_result() {
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let calls = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let root = cpu_graph(calls.clone(), Some(stop.clone()));
    let query = context
        .request(
            root.clone(),
            Region {
                bounds: [2, 1, 2, 2].into(),
                grid: root.domain().grid,
            },
            Budget {
                tile: [1, 1],
                scratch_bytes: 1 << 20,
            },
        )
        .unwrap();
    let mut published = false;
    let result = context
        .read(
            &queue,
            &Default::default(),
            &query,
            || stop.load(Ordering::Acquire),
            |_, _| {
                published = true;
                Ok(())
            },
        )
        .unwrap();
    assert!(result.is_none());
    assert!(!published);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[derive(Debug, PartialEq)]
struct Offset(f64);
impl Operation for Offset {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(Offset(self.0)))
    }
    fn reads(&self, output: CellRegion) -> Result<Vec<Read>, crate::Error> {
        Ok(vec![Read::new(0, output)])
    }
    fn calculation(
        self: std::sync::Arc<Self>,
        inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _budget: crate::gpu::evaluation::Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, crate::Error> {
        let inputs = crate::gpu::evaluation::value::Calculation::references(inputs);
        let function=Function::new("@group(0) @binding(0) var<uniform> parameters:vec4f; fn offset(p:vec4f, at:vec2f)->vec4f { return vec4f(p.rgb+vec3f(parameters.x),p.w); }", "offset")
            .with_uniform(0, &[self.0 as f32,0.,0.,0.]);
        value::Calculation::new(
            domain,
            output,
            vec![inputs[0].request(output)?],
            value::Operation::shader_function(self.key(), function),
        )
        .map(Some)
    }
}
struct Merge;
impl Operation for Merge {
    fn reads(&self, output: CellRegion) -> Result<Vec<Read>, crate::Error> {
        Ok(vec![Read::new(0, output), Read::new(1, output)])
    }
    fn calculation(
        self: std::sync::Arc<Self>,
        inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _: Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
        let inputs = crate::gpu::evaluation::value::Calculation::references(inputs);
        let operands = inputs
            .iter()
            .map(|input| input.request(output))
            .collect::<Result<Vec<_>, _>>()?;
        let operation = value::Operation::new(
            "weighted merge",
            (),
            "fn pixel(p:vec2i,position:vec2f,a:vec4f,b:vec4f)->vec4f{return a+2.0*b;}",
            "pixel",
            value::Access::Pointwise,
        );
        value::Calculation::new(domain, output, operands, operation).map(Some)
    }
}

#[test]
fn independent_cpu_dependencies_are_fulfilled_before_intervening_gpu_commands() {
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 1 << 20,
        },
    );
    let a_calls = Arc::new(AtomicUsize::new(0));
    let b_calls = Arc::new(AtomicUsize::new(0));
    let a = cpu_graph(a_calls.clone(), None);
    let original = source(Arc::new(Ramp));
    let triple = Value::from_operation(original.domain(), [original], Triple);
    let shifted = Value::from_operation(triple.domain(), [triple], Offset(10.));
    let b = Value::from_operation(
        a.domain(),
        [shifted],
        WholeCpu {
            region: a.domain().finite().unwrap(),
            calls: b_calls.clone(),
            stop: None,
        },
    );
    let b = pointwise(
        b,
        Meaning::Neutral,
        Function::new(
            "fn finish(p:vec4f, at:vec2f)->vec4f { return p+vec4f(1.,2.,3.,0.); }",
            "finish",
        ),
    );
    let root = Value::from_operation(a.domain(), [a, b], Merge);
    let region = CellRegion::new([2, 1], [2, 2]);
    let query = context
        .request(
            root.clone(),
            Region {
                bounds: region,
                grid: root.domain().grid,
            },
            Budget {
                tile: [2, 2],
                scratch_bytes: 1 << 20,
            },
        )
        .unwrap();
    let mut values = Vec::new();
    context
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
    assert_eq!(
        values,
        vec![
            [7317., 273., 69., 0.],
            [7344., 273., 69., 0.],
            [7317., 300., 69., 0.],
            [7344., 300., 69., 0.]
        ]
    );
    assert_eq!(a_calls.load(Ordering::Relaxed), 1);
    assert_eq!(b_calls.load(Ordering::Relaxed), 1);
}

#[test]
fn nested_cpu_readback_keeps_the_requested_materialization_scope() {
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 1 << 20,
        },
    );
    let root = cpu_graph(Arc::new(AtomicUsize::new(0)), None);
    let region = CellRegion::new([2, 1], [1, 1]);
    let query = context
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
        .unwrap();
    let isolated = query
        .clone()
        .with_materializations(context.test_materializations(false));
    context
        .read(
            &queue,
            &Default::default(),
            &isolated,
            || false,
            |_, _| Ok(()),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        context.retained_bytes(),
        0,
        "an isolated query must not publish into the context's default store"
    );
    context
        .read(&queue, &Default::default(), &query, || false, |_, _| Ok(()))
        .unwrap()
        .unwrap();
    assert!(
        context.retained_bytes() > 0,
        "the default store is enabled: the isolation assertion is not a disabled-cache no-op"
    );
}

#[test]
fn an_owned_gpu_unit_materialization_executes_without_joining_its_own_pending_result() {
    struct UniformPlate;
    impl Source for UniformPlate {
        fn domain(&self) -> Domain {
            Ramp.domain()
        }
        fn identity(&self) -> Option<RetainedKey> {
            Some(RetainedKey::new("owned-unit constant signed HDR plate"))
        }
        fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, crate::Error> {
            Ok(vec![
                [2., -1., 0.25, 0.];
                region.width as usize * region.height as usize
            ])
        }
    }
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let region = CellRegion::new([0, 0], [4, 1]);
    let root = reconstruct(
        source(Arc::new(UniformPlate)),
        Domain {
            bounds: Some(region),
            grid: Grid::axis_aligned([0.; 2], [250., 20.]),
            meaning: Meaning::Neutral,
        },
    )
    .unwrap();
    let allowance = Budget {
        tile: [4, 1],
        scratch_bytes: 32 << 10,
    };
    let unit = root
        .plan_unit(region, allowance)
        .unwrap()
        .admit_for(allowance, &device.limits())
        .unwrap();
    assert!(
        matches!(unit.work, UnitWork::Gpu(ref gpu) if !gpu.state.is_empty()),
        "the fixture must require owned GPU state, not only a fused expression"
    );
    let query = context
        .request(
            root.clone(),
            Region {
                bounds: region,
                grid: root.domain().grid,
            },
            allowance,
        )
        .unwrap();
    let value = pollster::block_on(
        context
            .materialize(
                &queue,
                &Default::default(),
                &context.materializations(),
                query,
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        read_buffer(&device, &queue, value.as_buffer().unwrap()),
        vec![[2., -1., 0.25, 0.]; 4]
    );
}
