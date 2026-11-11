use super::*;
mod callable;
mod dusty;
mod regional;
use crate::dust_repair::{RepairId, RepairOrigin};
use std::sync::Mutex;

struct Plate(Arc<Mutex<Vec<CellRegion>>>);
impl Source for Plate {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some([0, 0, 96, 64].into()),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::Neutral,
        }
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("signed HDR plate"))
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        self.0.lock().unwrap().push(region);
        Ok(vec![
            [2., -0.5, 0.25, 0.75];
            region.width as usize * region.height as usize
        ])
    }
}
struct Damage;
impl Operation for Damage {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("NaN at original cell 32,32"))
    }
    fn reads(&self, output: CellRegion) -> Result<Vec<Read>, Error> {
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
        let function = Function::new(
            "fn damage(p:vec4f, at:vec2f)->vec4f {\n\
            if all(vec2i(floor(at)) == vec2i(32,32)) { return vec4f(bitcast<f32>(0x7fc00000u), p.yzw); }\n\
            return p; }",
            "damage",
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
fn base(reads: Arc<Mutex<Vec<CellRegion>>>) -> Value {
    let source = source(Arc::new(Plate(reads)));
    Value::from_operation(source.domain(), [source], Damage)
}
fn repair(point: [u32; 2], offset: [i32; 2]) -> Repair {
    let mut bits = vec![false; 9];
    bits[4] = true;
    Repair {
        id: RepairId(1),
        enabled: true,
        origin: RepairOrigin::Manual,
        support: SupportMask::new([point[0] - 1, point[1] - 1], [3, 3], bits).unwrap(),
        offset,
    }
}
fn pixels(
    context: &Context,
    queue: &wgpu::Queue,
    image: Value,
    region: CellRegion,
) -> Result<Vec<[f32; 4]>, Error> {
    pixels_with_policy(context, queue, image, region, Policy::Composed)
}

fn pixels_with_policy(
    context: &Context,
    queue: &wgpu::Queue,
    image: Value,
    region: CellRegion,
    policy: Policy,
) -> Result<Vec<[f32; 4]>, Error> {
    let query = context
        .request(
            image.clone(),
            Region {
                bounds: region,
                grid: image.domain().grid,
            },
            Budget {
                tile: [2, 2],
                scratch_bytes: 1 << 20,
            },
        )?
        .with_policy(policy);
    let mut pixels = vec![[0.; 4]; region.width as usize * region.height as usize];
    context
        .read(
            queue,
            &Default::default(),
            &query,
            || false,
            |block, values| {
                for row in 0..block.height {
                    let from = row as usize * block.width as usize;
                    let to = region.index([block.x, block.y + row as i32]);
                    pixels[to..to + block.width as usize]
                        .copy_from_slice(&values[from..from + block.width as usize]);
                }
                Ok(())
            },
        )?
        .ok_or(Error::Cancelled)?;
    Ok(pixels)
}

#[test]
fn healing_is_lazy_masks_unused_nan_cells_and_preserves_signed_hdr_and_metadata() {
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 1 << 20,
        },
    );
    let reads = Arc::new(Mutex::new(Vec::new()));
    let base = base(reads.clone());
    let healed = compose(base.clone(), &[repair([32, 32], [32, 0])]).unwrap();
    assert!(
        reads.lock().unwrap().is_empty(),
        "describing healing must not acquire pixels"
    );
    let raw = pixels(&context, &queue, base, [32, 32, 1, 1].into()).unwrap();
    assert!(
        raw[0][0].is_nan(),
        "the control must actually contain the damaged sample"
    );
    let graded = pointwise(
        healed.clone(),
        Meaning::Graded,
        Function::new(
            "fn twice(p:vec4f, at:vec2f)->vec4f { return vec4f(2.*p.rgb,p.w); }",
            "twice",
        ),
    );
    let result = pixels(&context, &queue, graded, [30, 30, 5, 5].into()).unwrap();
    assert_eq!(
        result,
        vec![[4., -1., 0.5, 0.75]; 25],
        "constant target/donor has zero boundary correction; only the marked NaN is replaced"
    );
    reads.lock().unwrap().clear();
    let another_grade = pointwise(
        healed,
        Meaning::Graded,
        Function::new(
            "fn thrice(p:vec4f, at:vec2f)->vec4f { return vec4f(3.*p.rgb,p.w); }",
            "thrice",
        ),
    );
    assert_eq!(
        pixels(&context, &queue, another_grade, [32, 32, 1, 1].into()).unwrap(),
        vec![[6., -1.5, 0.75, 0.75]]
    );
    assert!(
        reads
            .lock()
            .unwrap()
            .iter()
            .all(|region| *region == CellRegion::new([32, 32], [1, 1])),
        "a new downstream grade must not reacquire target/donor context for the completed solve"
    );
}

#[test]
fn separated_groups_in_one_repair_keep_their_target_and_donor_crops() {
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let mut bits = vec![false; 18 * 3];
    bits[19] = true;
    bits[35] = true;
    let repair = Repair {
        id: RepairId(1),
        enabled: true,
        origin: RepairOrigin::Manual,
        support: SupportMask::new([31, 31], [18, 3], bits).unwrap(),
        offset: [32, 0],
    };
    let healed = compose(base(Default::default()), &[repair]).unwrap();
    assert_eq!(
        pixels(&context, &queue, healed, [30, 30, 21, 5].into()).unwrap(),
        vec![[2., -0.5, 0.25, 0.75]; 105],
        "each group has zero boundary correction on the constant plate; intervening cells stay unchanged"
    );
}

#[test]
fn noncontributing_source_edge_repair_does_not_run_its_invalid_solve() {
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let mut bits = vec![false; 16 * 16];
    bits[0] = true;
    bits[16 * 16 - 1] = true;
    let edge = Repair {
        id: RepairId(1),
        enabled: true,
        origin: RepairOrigin::Manual,
        support: SupportMask::new([0, 0], [16, 16], bits).unwrap(),
        offset: [32, 0],
    };
    let healed = compose(base(Default::default()), &[edge]).unwrap();
    assert!(
        pixels(&context, &queue, healed.clone(), [0, 0, 1, 1].into()).is_err(),
        "the control must require unavailable two-pixel exterior if executed"
    );
    assert_eq!(
        pixels(&context, &queue, healed.clone(), [8, 8, 1, 1].into()).unwrap(),
        vec![[2., -0.5, 0.25, 0.75]],
        "a hole inside the support's bounding rectangle does not require the invalid whole solve"
    );
    assert_eq!(
        pixels(&context, &queue, healed, [40, 32, 1, 1].into()).unwrap(),
        vec![[2., -0.5, 0.25, 0.75]]
    );
}

#[test]
fn rotated_reconstruction_skips_zero_weight_source_edge_repair_but_requires_contributing_edges() {
    use crate::{
        crop::Crop,
        frame_raster::FrameRaster,
        geometry::{Composition, FramePoint},
    };
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    for policy in [Policy::Materialized, Policy::Composed] {
        let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
        let plate = source(Arc::new(Plate(Default::default())));
        let edge = Repair {
            id: RepairId(1),
            enabled: true,
            origin: RepairOrigin::Manual,
            support: SupportMask::new([0, 0], [1, 1], vec![true]).unwrap(),
            offset: [32, 0],
        };
        let healed = compose(plate.clone(), &[edge]).unwrap();
        let composition = Composition::with_crop(
            [0, 0, 96, 64].into(),
            Default::default(),
            Default::default(),
            Some(Crop {
                center: [8., 8.],
                size: [16, 4],
                angle: std::f64::consts::FRAC_PI_4,
            }),
        )
        .unwrap();
        let frame = FrameRaster {
            output: [1, 1],
            ..FrameRaster::full(composition).unwrap()
        };
        let bounds = frame.processing_bounds().unwrap();
        assert!(
            bounds.contains_region([0, 0, 1, 1].into()),
            "the bounding envelope must include the irrelevant edge cell"
        );
        let coverage = composition.coverage(frame.min, frame.max).unwrap();
        assert_eq!(
            coverage.rectangle_weight([0., 0.], [1., 1.]),
            0.,
            "the edge cell has no actual polygon weight"
        );
        let reconstruct = |input: Value, frame: FrameRaster| {
            crate::gpu::evaluation::reconstruct(
                input.clone(),
                Domain {
                    bounds: Some(CellRegion::new([0, 0], frame.output)),
                    grid: Grid::from_frame(frame),
                    meaning: input.domain().meaning,
                },
            )
            .unwrap()
        };
        let clean = pixels_with_policy(
            &context,
            &queue,
            reconstruct(plate, frame),
            [0, 0, 1, 1].into(),
            policy,
        )
        .unwrap();
        let actual = pixels_with_policy(
            &context,
            &queue,
            reconstruct(healed.clone(), frame),
            [0, 0, 1, 1].into(),
            policy,
        )
        .unwrap();
        assert_eq!(
            actual, clean,
            "a zero-weight invalid solve cannot poison the photograph"
        );
        for (min, max) in [([0., 0.], [2., 2.]), ([-8., -9.], [-3., -4.])] {
            let needed = FrameRaster {
                composition: Composition::new(
                    [0, 0, 96, 64].into(),
                    Default::default(),
                    Default::default(),
                )
                .unwrap(),
                min: FramePoint(min),
                max: FramePoint(max),
                output: [1, 1],
            };
            let error = pixels_with_policy(
                &context,
                &queue,
                reconstruct(healed.clone(), needed),
                [0, 0, 1, 1].into(),
                policy,
            )
            .unwrap_err();
            assert!(
                error.to_string().contains("two-pixel exterior"),
                "required source edges, including constant exterior extension, really execute: {error}"
            );
        }
    }
}

#[test]
fn zero_weight_repair_with_valid_read_plan_does_not_execute_its_poisoned_donor() {
    use crate::{crop::Crop, frame_raster::FrameRaster, geometry::Composition};
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let repaired = repair([2, 12], [30, 20]);
    let input = base(Default::default());
    let healed = compose(input.clone(), &[repaired]).unwrap();
    assert!(
        matches!(
            pixels(&context, &queue, healed.clone(), [2, 12, 1, 1].into()),
            Err(Error::NumericalOverflow)
        ),
        "the donor at32,32 is really poisoned"
    );
    let composition = Composition::with_crop(
        [0, 0, 96, 64].into(),
        Default::default(),
        Default::default(),
        Some(Crop {
            center: [8., 8.],
            size: [16, 4],
            angle: std::f64::consts::FRAC_PI_4,
        }),
    )
    .unwrap();
    let frame = FrameRaster {
        output: [1, 1],
        ..FrameRaster::full(composition).unwrap()
    };
    assert_eq!(
        composition
            .coverage(frame.min, frame.max)
            .unwrap()
            .rectangle_weight([2., 12.], [1.; 2]),
        0.
    );
    let root = crate::gpu::evaluation::reconstruct(
        healed,
        Domain {
            bounds: Some([0, 0, 1, 1].into()),
            grid: Grid::from_frame(frame),
            meaning: Meaning::Neutral,
        },
    )
    .unwrap();
    let actual = pixels(&context, &queue, root, [0, 0, 1, 1].into()).unwrap();
    assert_eq!(&actual[0][..3], &[2., -0.5, 0.25]);
}

#[test]
fn a_required_natural_heal_can_be_hoisted_without_deferring_the_whole_reconstruction() {
    let reads = Arc::new(Mutex::new(Vec::new()));
    let base = base(reads.clone());
    let repair = repair([32, 32], [32, 0]);
    let expected = healing_image(&base, &repair).unwrap();
    let expected = expected
        .request(expected.domain().finite().unwrap())
        .unwrap();
    assert!(
        expected.identity().is_some(),
        "the fixture must identify the CPU calculation"
    );
    let healed = compose(base, &[repair]).unwrap();
    let region = CellRegion::new([0, 0], [5, 5]);
    let root = crate::gpu::evaluation::reconstruct(
        healed,
        Domain {
            bounds: Some(region),
            grid: Grid::axis_aligned([30., 30.], [1.; 2]),
            meaning: Meaning::Neutral,
        },
    )
    .unwrap();
    let plan = Plan::with_policy(
        root.clone(),
        region,
        Budget {
            tile: [5, 5],
            scratch_bytes: 1 << 20,
        },
        Policy::Composed,
    )
    .unwrap();
    let deferred = plan
        .tiles
        .iter()
        .flat_map(|tile| &tile.fragment.work)
        .filter_map(|work| match work {
            Work::Produce { image, output } => Some((image, output.region)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        !deferred.is_empty(),
        "the cold healer really needs worker execution"
    );
    let natural = expected.coverage();
    assert!(
        deferred.iter().all(|(image, output)| {
            !image.same_definition(&root)
                && u64::from(output.width) * u64::from(output.height)
                    == u64::from(natural.width) * u64::from(natural.height)
                && image
                    .ancestors()
                    .any(|parent| parent.identity() == expected.identity())
        }),
        "only the actually required whole heal and its selected arguments are hoisted, not enclosing image work"
    );
    assert!(
        reads.lock().unwrap().is_empty(),
        "the compiler must not acquire pixels to prove the natural unit is required"
    );
}

#[test]
fn a_repair_donor_reads_unrepaired_neutral_not_an_earlier_healed_sibling() {
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let mut second = repair([64, 32], [-32, 0]);
    second.id = RepairId(2);
    let healed = compose(
        base(Default::default()),
        &[repair([32, 32], [32, 0]), second],
    )
    .unwrap();
    assert!(
        matches!(
            pixels(&context, &queue, healed, [64, 32, 1, 1].into()),
            Err(Error::NumericalOverflow)
        ),
        "a repaired sibling must not hide the NaN in this repair's actual donor input"
    );
}
