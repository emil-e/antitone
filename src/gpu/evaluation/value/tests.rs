use super::*;
mod admission;
mod contracts;
mod coordinates;
mod copies;
mod definitions;
mod demands;
mod functions;
mod graph;
mod refinement;
mod reuse;
mod tables;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone)]
struct Plate {
    reads: Arc<AtomicUsize>,
    id: usize,
    domain: Domain,
}
impl Plate {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        Self {
            reads: Arc::new(AtomicUsize::new(0)),
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            domain: Domain {
                bounds: Some(CellRegion::new([-5, 3], [21, 15])),
                grid: Grid::axis_aligned([2., -1.], [0.5, 2.]),
                meaning: Meaning::Graded,
            },
        }
    }
    fn pixel(&self, p: [i32; 2]) -> [f32; 4] {
        [
            p[0] as f32 * 0.25 - 2.,
            p[1] as f32 * 0.5 + 3.,
            if p == [4, 10] { 20. } else { 0.25 },
            if self.domain.grid.coverage != 0 {
                u8::from(p[1] != 8 && (p[0] + p[1]).rem_euclid(5) != 0) as f32
            } else {
                0.5 + (p[0] + p[1]) as f32 * 1e-5
            },
        ]
    }
}
impl Source for Plate {
    fn domain(&self) -> Domain {
        self.domain
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(("value plate", self.id)))
    }
    fn read(&self, r: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        assert!(self.domain.contains_region(r));
        self.reads.fetch_add(1, Ordering::Relaxed);
        Ok((r.y..r.bottom())
            .flat_map(|y| (r.x..r.right()).map(move |x| self.pixel([x, y])))
            .collect())
    }
}
const AFFINE: &str = r#"
struct Parameters { affine: vec4f }
fn pixel(p: vec2i, position: vec2f, original: vec4f, parameters: Parameters) -> vec4f {
    return vec4f(original.rgb * parameters.affine.x + parameters.affine.y + position.x * 0.01, original.w);
}
"#;
fn affine(input: Value, gain: f64, offset: f64) -> Value {
    pointwise(
        input,
        Meaning::Graded,
        Operation::new(
            "affine position v1",
            (gain, offset),
            AFFINE,
            "pixel",
            Access::Pointwise,
        )
        .with_uniform("Parameters", &[gain as f32, offset as f32, 0., 0.]),
    )
    .unwrap()
}
fn budget(r: CellRegion) -> Budget {
    Budget {
        tile: [r.width, r.height],
        scratch_bytes: 32 << 20,
    }
}
fn run(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    value: &Value,
    policy: Policy,
) -> (Vec<[f32; 4]>, WorkCounts) {
    let plan = Plan::with_policy(
        value.clone(),
        value.coverage(),
        budget(value.coverage()),
        policy,
    )
    .unwrap();
    let result = Executor::default()
        .run(device, queue, &Default::default(), &plan, || false)
        .unwrap()
        .unwrap();
    (
        super::super::execute::read(device, queue, &result.pixels),
        result.counts,
    )
}

#[test]
fn one_pixel_body_runs_fused_or_materialized_with_private_parameters_and_coordinates() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let plate = Plate::new();
    let reads = plate.reads.clone();
    let region = plate.domain.finite().unwrap();
    let root = affine(affine(source(Arc::new(plate.clone())), 2., 1.), -0.5, 0.25);
    let value = root.request(region).unwrap();
    assert_eq!(
        reads.load(Ordering::Relaxed),
        0,
        "describing values must not obtain pixels"
    );
    let mut counts = Vec::new();
    for policy in [Policy::Materialized, Policy::Composed] {
        let (pixels, physical) = run(&device, &queue, &value, policy);
        assert_eq!(pixels.len(), (region.width * region.height) as usize);
        counts.push(physical);
        for (index, pixel) in pixels.iter().enumerate() {
            let p = [
                region.x + (index % region.width as usize) as i32,
                region.y + (index / region.width as usize) as i32,
            ];
            let original = plate.pixel(p);
            let x = plate.domain.point([p[0] as f64 + 0.5, p[1] as f64 + 0.5])[0];
            for c in 0..3 {
                let expected = -f64::from(original[c]) - 0.25 + x * 0.005;
                assert!(
                    (f64::from(pixel[c]) - expected).abs() < 2e-5,
                    "{p:?}: {pixel:?} expected {expected}"
                );
            }
            assert_eq!(pixel[3].to_bits(), original[3].to_bits());
        }
    }
    assert!(
        counts[1].buffer_allocations < counts[0].buffer_allocations,
        "fusion must avoid actual intermediate allocations"
    );
}

#[test]
fn matching_value_keys_reuse_supersets_and_pass_through_without_source_reads() {
    struct Identity;
    impl super::super::Operation for Identity {
        fn describe(
            &self,
            output: Output,
            inputs: &[crate::gpu::evaluation::value::Value],
            _requested: &[CellRegion],
        ) -> Result<Output, Error> {
            inputs[0].output(output.region)
        }

        fn calculation(
            self: std::sync::Arc<Self>,
            inputs: &[crate::gpu::evaluation::value::Value],
            _: Domain,
            r: CellRegion,
            _budget: crate::gpu::evaluation::Budget,
        ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
            let inputs = crate::gpu::evaluation::value::Calculation::references(inputs);
            Ok(Some(inputs[0].request(r)?))
        }
    }
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 4 << 20,
        },
    );
    let plate = Plate::new();
    let reads = plate.reads.clone();
    let full = plate.domain.finite().unwrap();
    let node = affine(source(Arc::new(plate.clone())), 1.5, 0.25);
    let big = node.request(full).unwrap();
    let small = CellRegion::new([-2, 6], [5, 4]);
    let small_value = node.request(small).unwrap();
    assert_eq!(
        big.identity(),
        small_value.identity(),
        "coverage is not a mathematical argument of this operation"
    );
    let gate = crate::gpu::submission::SubmissionGate::default();
    let scope = context.materializations();
    let first = pollster::block_on(
        context
            .materialize(
                &queue,
                &gate,
                &scope,
                context
                    .request(
                        node.clone(),
                        Region {
                            bounds: full,
                            grid: node.domain().grid,
                        },
                        budget(full),
                    )
                    .unwrap(),
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap();
    let before = reads.load(Ordering::Relaxed);
    assert!(before > 0);
    let pass = Value::from_operation(node.domain(), [node], Identity);
    let (output, descriptions) = testing::observe(0, || pass.output(small).unwrap());
    assert_eq!(descriptions, 0);
    assert_eq!(output.identity(), small_value.identity());
    let returned = pass.request(small).unwrap();
    assert_eq!(returned.identity(), small_value.identity());
    let second = pollster::block_on(
        context
            .materialize(
                &queue,
                &gate,
                &scope,
                context
                    .request(
                        pass.clone(),
                        Region {
                            bounds: small,
                            grid: pass.domain().grid,
                        },
                        budget(small),
                    )
                    .unwrap(),
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap();
    assert!(Arc::ptr_eq(&first.image, &second.image));
    assert_eq!(second.descriptor.region, full);
    assert_eq!(reads.load(Ordering::Relaxed), before);
    let (pixels, stats) = run_cached(&context, &queue, &gate, &pass, small);
    for (index, pixel) in pixels.iter().enumerate() {
        let p = [
            small.x + (index % small.width as usize) as i32,
            small.y + (index / small.width as usize) as i32,
        ];
        let base = plate.pixel(p);
        let x = plate.domain.point([p[0] as f64 + 0.5, p[1] as f64 + 0.5])[0];
        for c in 0..3 {
            let expected = f64::from(base[c]) * 1.5 + 0.25 + x * 0.01;
            assert!(
                (f64::from(pixel[c]) - expected).abs() < 2e-5,
                "cached subregion {p:?}: {pixel:?}, expected {expected}"
            );
        }
        assert_eq!(pixel[3].to_bits(), base[3].to_bits());
    }
    assert_eq!(pixels.len(), (small.width * small.height) as usize);
    assert_eq!(
        stats.source_cells, 0,
        "the cached containing value must be consumed, not merely discoverable"
    );
    assert_eq!(reads.load(Ordering::Relaxed), before);
}
fn run_cached(
    context: &Context,
    queue: &wgpu::Queue,
    gate: &crate::gpu::submission::SubmissionGate,
    node: &Value,
    coverage: CellRegion,
) -> (Vec<[f32; 4]>, WorkCounts) {
    let evaluation = context
        .request(
            node.clone(),
            Region {
                bounds: coverage,
                grid: node.domain().grid,
            },
            budget(coverage),
        )
        .unwrap();
    let mut pixels = Vec::new();
    let counts = context
        .read(
            queue,
            gate,
            &evaluation,
            || false,
            |_, block| {
                pixels.extend_from_slice(block);
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    (pixels, counts)
}

#[test]
fn gaussian_group_expands_reads_and_preserves_source_edges_without_two_shader_implementations() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let plate = Plate::new();
    let reads = plate.reads.clone();
    let sigma = 0.65;
    let region = CellRegion::new([-5, 3], [12, 11]);
    let root = gaussian(source(Arc::new(plate.clone())), sigma).unwrap();
    let value = root.request(region).unwrap();
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    let source_bounds = plate.domain.finite().unwrap();
    let radii = plate
        .domain
        .grid
        .pitch
        .map(|pitch| (4. * sigma / pitch).ceil() as i32);
    let mut changed = false;
    for policy in [Policy::Materialized, Policy::Composed] {
        let (actual, _) = run(&device, &queue, &value, policy);
        assert_eq!(actual.len(), (region.width * region.height) as usize);
        for (index, pixel) in actual.iter().enumerate() {
            let p = [
                region.x + (index % region.width as usize) as i32,
                region.y + (index / region.width as usize) as i32,
            ];
            let mut expected = [0.; 3];
            let mut mass = 0.;
            for dy in -radii[1]..=radii[1] {
                for dx in -radii[0]..=radii[0] {
                    let q = [
                        (p[0] + dx).clamp(source_bounds.x, source_bounds.right() - 1),
                        (p[1] + dy).clamp(source_bounds.y, source_bounds.bottom() - 1),
                    ];
                    let weights = [dx, dy]
                        .into_iter()
                        .zip(plate.domain.grid.pitch)
                        .map(|(d, pitch)| (-0.5 * (f64::from(d) * pitch / sigma).powi(2)).exp())
                        .product::<f64>();
                    let sample = plate.pixel(q);
                    for c in 0..3 {
                        expected[c] += f64::from(sample[c]) * weights;
                    }
                    mass += weights;
                }
            }
            for c in 0..3 {
                expected[c] /= mass;
                assert!(
                    (f64::from(pixel[c]) - expected[c]).abs() < 3e-5,
                    "{p:?}/{c}: {pixel:?}, {expected:?}"
                );
            }
            changed |= (f64::from(plate.pixel(p)[2]) - expected[2]).abs() > 0.1;
            assert_eq!(pixel[3].to_bits(), plate.pixel(p)[3].to_bits());
        }
    }
    assert!(
        changed,
        "the impulse control must distinguish filtering from a pass-through"
    );
}

#[test]
fn materialized_chain_recycles_retired_scratch_without_overwriting_active_inputs() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let plate = Plate::new();
    let mut image = source(Arc::new(plate.clone()));
    let steps = [(1.1, -0.2), (0.7, 0.3), (1.3, -0.5), (0.9, 0.15)];
    for (gain, offset) in steps {
        image = affine(image, gain, offset);
    }
    let region = plate.domain.finite().unwrap();
    let plan = Plan::with_policy(image, region, budget(region), Policy::Materialized).unwrap();
    assert_eq!(plan.tiles.len(), 1);
    let tile = &plan.tiles[0];
    assert_eq!(
        tile.fragment
            .work
            .iter()
            .filter(|work| matches!(work, Work::Compute { .. }))
            .count(),
        steps.len()
    );
    let images = |resources: &[Storage]| {
        resources
            .iter()
            .filter(|resource| matches!(resource, Storage::Buffer(_)))
            .count()
    };
    assert!(
        images(&tile.slots) < images(&tile.fragment.resources),
        "the fixture must actually recycle a retired image allocation"
    );
    assert!(plan.peak_scratch_bytes <= budget(region).scratch_bytes);
    let execution = execute::Executor::default()
        .run(&device, &queue, &Default::default(), &plan, || false)
        .unwrap()
        .unwrap();
    let actual = execute::read(&device, &queue, &execution.pixels);
    for (index, pixel) in actual.iter().enumerate() {
        let cell = [
            region.x + (index as u32 % region.width) as i32,
            region.y + (index as u32 / region.width) as i32,
        ];
        let original = plate.pixel(cell);
        let position = plate
            .domain
            .point(cell.map(|coordinate| f64::from(coordinate) + 0.5));
        let mut expected = original.map(f64::from);
        for (gain, offset) in steps {
            for value in &mut expected[..3] {
                *value =
                    *value * f64::from(gain as f32) + f64::from(offset as f32) + position[0] * 0.01;
            }
        }
        for channel in 0..3 {
            assert!(
                (f64::from(pixel[channel]) - expected[channel]).abs() < 2e-5,
                "{cell:?}/{channel}: {pixel:?}, expected {expected:?}"
            );
        }
        assert_eq!(pixel[3].to_bits(), original[3].to_bits());
    }
}

#[test]
fn gaussian_opposite_extreme_signs_use_a_finite_convex_mean_and_preserve_metadata() {
    struct Extremes;
    impl Source for Extremes {
        fn domain(&self) -> Domain {
            Domain {
                bounds: Some([0, 0, 2, 1].into()),
                grid: Grid::axis_aligned([0.; 2], [1.; 2]),
                meaning: Meaning::Graded,
            }
        }
        fn identity(&self) -> Option<RetainedKey> {
            Some(RetainedKey::new("opposite extreme Gaussian witness"))
        }
        fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
            Ok((region.x..region.right())
                .map(|x| {
                    let a = f32::MAX * 0.75;
                    let sign = if x == 0 { 1. } else { -1. };
                    [a * sign, -a * sign, a, if x == 0 { -0. } else { 0.75 }]
                })
                .collect())
        }
    }
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let amplitude = f32::MAX * 0.75;
    assert!((amplitude - (-amplitude)).is_infinite());
    let sigma: f64 = 0.8;
    let radius = (4. * sigma).ceil() as i32;
    let root = gaussian(source(Arc::new(Extremes)), sigma).unwrap();
    let value = root.request([0, 0, 2, 1].into()).unwrap();
    for policy in [Policy::Materialized, Policy::Composed] {
        let (actual, _) = run(&device, &queue, &value, policy);
        assert_eq!(actual.len(), 2);
        for (x, pixel) in actual.iter().enumerate() {
            let mut sum = [0.; 3];
            let mut mass = 0.;
            for offset in -radius..=radius {
                let weight = (-0.5 * (f64::from(offset) / sigma).powi(2)).exp();
                let at = (x as i32 + offset).clamp(0, 1);
                let sample = Extremes.read([at, 0, 1, 1].into()).unwrap()[0];
                for channel in 0..3 {
                    sum[channel] += f64::from(sample[channel]) * weight;
                }
                mass += weight;
            }
            assert!(pixel.iter().all(|component| component.is_finite()));
            for channel in 0..3 {
                let expected = sum[channel] / mass;
                assert!(
                    (f64::from(pixel[channel]) - expected).abs() / f64::from(amplitude) < 2e-6,
                    "{x}/{channel}: {pixel:?}, expected {expected}"
                );
            }
            let original = Extremes.read([x as i32, 0, 1, 1].into()).unwrap()[0];
            assert_eq!(pixel[3].to_bits(), original[3].to_bits());
            assert!(
                (pixel[0] - original[0]).abs() > amplitude * 0.1,
                "the opposite-sign fixture must distinguish filtering from pass-through"
            );
        }
    }
}
