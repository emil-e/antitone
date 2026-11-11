use super::*;
use crate::gpu::evaluation::{self as e, Operation as GroupOperation};

struct Coordinates {
    domain: Domain,
    named: bool,
    cells: Arc<AtomicUsize>,
}
impl Source for Coordinates {
    fn domain(&self) -> Domain {
        self.domain
    }
    fn identity(&self) -> Option<RetainedKey> {
        self.named
            .then(|| RetainedKey::new(("refinement coordinates", self.domain)))
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        self.cells
            .fetch_add((region.width * region.height) as usize, Ordering::Relaxed);
        Ok((region.y..region.bottom())
            .flat_map(|y| {
                (region.x..region.right())
                    .map(move |x| [(x + 2 * y) as f32, (y - 4) as f32, (x - y) as f32, 0.125])
            })
            .collect())
    }
}
fn input(region: CellRegion, named: bool) -> (Value, Arc<AtomicUsize>) {
    let cells = Arc::new(AtomicUsize::new(0));
    let source = Arc::new(Coordinates {
        domain: Domain {
            bounds: Some(region),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::Graded,
        },
        named,
        cells: cells.clone(),
    });
    (e::source(source).request(region).unwrap(), cells)
}
fn collect(
    context: &Context,
    queue: &wgpu::Queue,
    value: Value,
    region: CellRegion,
    bytes: u64,
) -> Result<Vec<[f32; 4]>, Error> {
    let query = context.request(
        value.clone(),
        Region {
            bounds: region,
            grid: value.domain().grid,
        },
        Budget {
            tile: [region.width, region.height],
            scratch_bytes: bytes,
        },
    )?;
    let mut pixels = vec![[0.; 4]; (region.width * region.height) as usize];
    let mut seen = std::collections::HashSet::new();
    context
        .read(
            queue,
            &Default::default(),
            &query,
            || false,
            |block, values| {
                assert!(region.contains_region(block));
                assert_eq!(values.len(), (block.width * block.height) as usize);
                for (i, value) in values.iter().enumerate() {
                    let p = [
                        block.x + (i % block.width as usize) as i32,
                        block.y + (i / block.width as usize) as i32,
                    ];
                    assert!(seen.insert(p));
                    pixels[region.index(p)] = *value;
                }
                Ok(())
            },
        )?
        .unwrap();
    assert_eq!(seen.len(), pixels.len());
    Ok(pixels)
}

#[test]
fn directly_authored_constituents_refine_their_kernel_layouts_with_the_output() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let region = CellRegion::new([-3, 5], [8, 8]);
    let (source, cells) = input(region, false);
    let first = Value::new(source.domain(), region, vec![source], Operation::new(
        "coordinate shear", (), "fn pixel(p:vec2i, position:vec2f, a:vec4f)->vec4f { return vec4f(a.x+3.0*a.y, a.y-2.0*a.x, a.zw); }", "pixel", Access::Pointwise,
    )).unwrap();
    let root = Value::new(first.domain(), region, vec![first], Operation::new(
        "coordinate second shear", (), "fn pixel(p:vec2i, position:vec2f, a:vec4f)->vec4f { return vec4f(2.0*a.x-a.y, a.y+7.0, a.zw); }", "pixel", Access::Pointwise,
    )).unwrap();
    let allowance = Budget {
        tile: [8, 8],
        scratch_bytes: 768,
    };
    assert!(matches!(
        root.plan_unit(region, allowance).unwrap().admit(allowance),
        Err(Error::WorkingAllowance(_))
    ));
    let small = CellRegion::new([-3, 5], [2, 2]);
    let candidate = root
        .plan_unit(small, allowance)
        .unwrap()
        .admit(allowance)
        .unwrap();
    assert_eq!(candidate.output, small);
    assert_eq!(cells.load(Ordering::Relaxed), 0);
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let pixels = collect(&context, &queue, root, region, allowance.scratch_bytes).unwrap();
    for (i, pixel) in pixels.iter().enumerate() {
        let x = region.x + (i % region.width as usize) as i32;
        let y = region.y + (i / region.width as usize) as i32;
        assert_eq!(
            *pixel,
            [
                (4 * x + 13 * y - 20) as f32,
                (-2 * x - 3 * y + 3) as f32,
                (x - y) as f32,
                0.125
            ]
        );
    }
    assert!(cells.load(Ordering::Relaxed) >= pixels.len());
}

struct LazyPair;
impl GroupOperation for LazyPair {
    fn calculation(
        self: Arc<Self>,
        inputs: &[Value],
        domain: Domain,
        region: CellRegion,
        _: Budget,
    ) -> Result<Option<Calculation>, Error> {
        let inputs = Calculation::references(inputs);
        Calculation::new(domain, region, inputs, Operation::new(
            "lazy private pair", (), "fn pixel(p:vec2i, position:vec2f, a:vec4f)->array<vec4f,2> { return array<vec4f,2>(a,a); }", "pixel", Access::Pointwise,
        ).with_shape(Shape::Pair)).map(Some)
    }
}
#[test]
fn lazy_groups_cannot_deliver_private_records_as_photographic_pixels() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let region = CellRegion::new([0, 0], [4, 4]);
    let (source, cells) = input(region, false);
    let value = Value::captured(source.domain(), vec![source], Arc::new(LazyPair))
        .request(region)
        .unwrap();
    assert!(
        value.shape() == Shape::Pixel,
        "group shape is not yet lowered"
    );
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    assert!(matches!(
        collect(&context, &queue, value, region, 1 << 20),
        Err(Error::InvalidParameter(
            "record values cannot be requested as photographic images"
        ))
    ));
    assert_eq!(cells.load(Ordering::Relaxed), 0);
}

struct SplitWindows;
impl GroupOperation for SplitWindows {
    fn calculation(
        self: Arc<Self>,
        inputs: &[Value],
        _domain: Domain,
        _output: CellRegion,
        _: Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
        let region = inputs[0].domain().finite()?;
        let calculated = Calculation::new(
            inputs[0].domain(),
            region,
            vec![Calculation::input(inputs[0].request(region)?)],
            Operation::new(
                "window shared calculation",
                (),
                "fn pixel(p:vec2i, position:vec2f, a:vec4f)->vec4f { return vec4f(a.x*3.0, a.yz, a.w); }",
                "pixel",
                Access::Pointwise,
            ),
        )?;
        let mut halves = [region; 2];
        halves[0].width /= 2;
        halves[1].x += halves[0].width as i32;
        halves[1].width -= halves[0].width;
        let windows = halves
            .into_iter()
            .map(|part| {
                Calculation::window(
                    Domain {
                        bounds: Some(part),
                        ..calculated.domain()
                    },
                    part,
                    calculated.request(part)?,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let result = Calculation::concatenate(windows)?;
        Ok(Some(result))
    }
}
#[test]
fn shared_constituents_keep_each_window_stride_when_concatenated() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let region = CellRegion::new([-3, 5], [8, 4]);
    let (source, cells) = input(region, false);
    let flat = CellRegion::new([0, 0], [region.width * region.height, 1]);
    let value = Value::captured(
        Domain {
            bounds: Some(flat),
            meaning: Meaning::Arguments,
            grid: source.domain().grid,
        },
        vec![source],
        Arc::new(SplitWindows),
    )
    .request(flat)
    .unwrap();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let pixels = collect(&context, &queue, value, flat, 16 << 10).unwrap();
    let expected = [region.x, region.x + 4]
        .into_iter()
        .flat_map(|left| {
            (region.y..region.bottom()).flat_map(move |y| {
                (left..left + 4).map(move |x| {
                    [
                        (3 * (x + 2 * y)) as f32,
                        (y - 4) as f32,
                        (x - y) as f32,
                        0.125,
                    ]
                })
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(pixels, expected);
    assert_eq!(cells.load(Ordering::Relaxed), pixels.len());
}

#[test]
fn disconnected_private_record_demands_reject_unaddressable_scratch_before_source_io() {
    let huge = CellRegion::new([0, 0], [i32::MAX as u32; 2]);
    let (source, cells) = input(huge, false);
    let record = Value::new(source.domain(), huge, vec![], Operation::new(
        "coordinate records", (), "fn pixel(p:vec2i, position:vec2f)->array<vec4f,2> { return array<vec4f,2>(vec4f(position,0.0,1.0),vec4f(0.0)); }", "pixel", Access::Pointwise,
    ).with_shape(Shape::Pair)).unwrap();
    let output = CellRegion::new([0, 0], [1, 1]);
    let distant = CellRegion::new([i32::MAX - 1; 2], [1, 1]);
    let views = [output, distant].into_iter().map(|region| Value::new(record.domain(), region, vec![record.request(region).unwrap()], Operation::new(
        "record views", (), "fn pixel(p:vec2i, position:vec2f, a:array<vec4f,2>)->array<vec4f,2> { return a; }", "pixel", Access::Pointwise,
    ).with_shape(Shape::Pair)).unwrap()).collect::<Vec<_>>();
    let root = Value::new(record.domain(), output, vec![views[0].clone(), views[1].clone(), source.request(output).unwrap()], Operation::new(
        "disconnected record extraction", (), "fn pixel(p:vec2i, position:vec2f)->vec4f { return input_0(vec2i(0))[0] + input_1(vec2i(2147483646))[0] + input_2(p); }", "pixel", Access::Neighborhood,
    )).unwrap();
    // Each operand reads one cell, but their shared scratch rectangle spans both.
    assert!(matches!(
        root.plan_unit(output, budget(output)),
        Err(Error::WorkingAllowance(
            "unit state exceeds GPU cell addressing"
        ))
    ));
    assert_eq!(cells.load(Ordering::Relaxed), 0);
}
