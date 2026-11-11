use super::*;
use crate::gpu::evaluation::Operation as ImageOperation;

fn expression(source: Value, domain: Domain, region: CellRegion, steps: usize) -> Value {
    expression_nodes(
        source.request(region).unwrap(),
        domain,
        region,
        steps,
        Value::new,
    )
    .unwrap()
}
fn expression_nodes<T: Clone>(
    source: T,
    domain: Domain,
    region: CellRegion,
    steps: usize,
    new: impl Fn(Domain, CellRegion, Vec<T>, Operation) -> Result<T, Error>,
) -> Result<T, Error> {
    let arguments = Domain {
        meaning: Meaning::Arguments,
        ..domain
    };
    let seed=new(arguments,region,vec![source],
        Operation::new("paired sample witness v1",(),"fn pixel(p:vec2i, at:vec2f, value:vec4f)->array<vec4f,2>{return array<vec4f,2>(vec4f(value.rgb,1.0),vec4f(value.w,0.0,0.0,0.0));}","pixel",Access::Pointwise).with_shape(Shape::Pair))?;
    let mut current = seed.clone();
    for _ in 0..steps {
        current=new(arguments,region,vec![current],Operation::new("paired increment witness v1",(),
            "fn pixel(p:vec2i,at:vec2f,value:array<vec4f,2>)->array<vec4f,2>{return array<vec4f,2>(vec4f(value[0].rgb+vec3f(1.0,-2.0,3.0),value[0].w),value[1]);}","pixel",Access::Pointwise).with_shape(Shape::Pair))?;
    }
    let combined=new(arguments,region,vec![seed,current],Operation::new("paired branch witness v1",(),
        "fn pixel(p:vec2i,at:vec2f,original:array<vec4f,2>,value:array<vec4f,2>)->array<vec4f,2>{return array<vec4f,2>(vec4f(original[0].rgb+value[0].rgb,1.0),original[1]);}","pixel",Access::Pointwise).with_shape(Shape::Pair))?;
    new(
        domain,
        region,
        vec![combined],
        Operation::new(
            "paired photographic output witness v1",
            (),
            "fn pixel(p:vec2i,at:vec2f,value:array<vec4f,2>)->vec4f{return vec4f(value[0].rgb,value[1].x);}",
            "pixel",
            Access::Pointwise,
        ),
    )
}

pub(super) struct Graph {
    pub(super) steps: usize,
    pub(super) plans: Arc<AtomicUsize>,
}
impl ImageOperation for Graph {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(("value graph witness", self.steps)))
    }
    fn work_units(&self, region: CellRegion, _: Budget) -> Result<WorkUnits, Error> {
        tiled_units(region, [2, 1])
    }
    fn calculation(
        self: Arc<Self>,
        inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _: Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
        self.plans.fetch_add(1, Ordering::Relaxed);
        let source = Calculation::input(inputs[0].request(output)?);
        let result = expression_nodes(source, domain, output, self.steps, Calculation::new)?;
        Ok(Some(result))
    }
}

#[test]
fn constituent_descriptions_do_not_author_values_or_parse_shader_libraries() {
    let plate = Plate::new();
    let reads = plate.reads.clone();
    let area = CellRegion::new([0, 6], [2, 1]);
    let root = Value::from_operation(
        plate.domain,
        [source(Arc::new(plate))],
        Graph {
            steps: 3,
            plans: Default::default(),
        },
    );
    for scratch_bytes in [8192, 16384] {
        let ((unit, definitions), libraries) =
            super::super::super::sampling::library_testing::observe(|| {
                testing::observe(0, || {
                    let budget = Budget {
                        tile: [2, 1],
                        scratch_bytes,
                    };
                    let described = root.lower(area, budget).unwrap();
                    described.compile(area, budget).unwrap()
                })
            });
        assert_eq!(
            definitions, 0,
            "lowering may describe instructions, not replacement mathematical Values"
        );
        assert_eq!(
            libraries,
            (0, 0),
            "unit admission precedes shader parsing/validation"
        );
        assert_eq!(unit.output, area);
        assert_eq!(reads.load(Ordering::Relaxed), 0);
    }
}

#[test]
fn record_value_fanout_keeps_live_branches_and_metadata_through_reused_gpu_storage() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let gate = crate::gpu::submission::SubmissionGate::default();
    let plate = Plate::new();
    let area = CellRegion::new([-4, 5], [3, 2]);
    let input = source(Arc::new(plate.clone()));
    let root = Value::from_operation(
        plate.domain,
        [input],
        Graph {
            steps: 8,
            plans: Default::default(),
        },
    );
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let query = context
        .request(
            root,
            Region {
                bounds: area,
                grid: plate.domain.grid,
            },
            Budget {
                tile: [3, 2],
                scratch_bytes: 8 << 10,
            },
        )
        .unwrap();
    let mut actual = Vec::new();
    context
        .read(
            &queue,
            &gate,
            &query,
            || false,
            |r, block| {
                assert_eq!(r, area);
                actual.extend_from_slice(block);
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(actual.len(), 6);
    for (i, pixel) in actual.iter().enumerate() {
        let p = [area.x + (i % 3) as i32, area.y + (i / 3) as i32];
        let base = plate.pixel(p);
        assert_eq!(
            &pixel[..3],
            &[base[0] * 2. + 8., base[1] * 2. - 16., base[2] * 2. + 24.]
        );
        assert_eq!(pixel[3].to_bits(), base[3].to_bits());
    }
}

#[test]
fn long_value_graphs_compile_and_retire_iteratively_with_scratch_bounded_by_live_values() {
    let plate = Plate::new();
    let reads = plate.reads.clone();
    let input = source(Arc::new(plate)).clone();
    let area = CellRegion::new([0, 6], [3, 2]);
    let unit = compile_gpu_graph(
        Calculation::from_value(expression(input.clone(), input.domain(), area, 4096)).unwrap(),
        std::slice::from_ref(&input),
    )
    .unwrap();
    assert_eq!(
        reads.load(Ordering::Relaxed),
        0,
        "compilation does not acquire external image boundaries"
    );
    assert!(
        unit.state.len() <= 3,
        "only the saved branch and two successive paired results can be live together"
    );
    assert!(
        unit.state.iter().sum::<u64>() <= 3 * 6 * 32,
        "scratch must not grow with the number of constituent operations"
    );
    drop(unit);
    let result = expression(input.clone(), input.domain(), area, 4096);
    let record = result.inputs()[0].request(area).unwrap();
    drop(result);
    assert!(matches!(
        compile_gpu_graph(Calculation::from_value(record).unwrap(), &[input]),
        Err(Error::InvalidParameter(
            "GPU image graph must return photographic pixels"
        ))
    ));
}
