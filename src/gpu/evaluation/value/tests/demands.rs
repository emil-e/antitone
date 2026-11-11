use super::*;
use std::sync::Mutex;

#[derive(Clone)]
struct WideRamp(Arc<Mutex<Vec<CellRegion>>>);
impl WideRamp {
    fn pixel(p: [i32; 2]) -> [f32; 4] {
        [
            p[0] as f32 / 1024. - 2.,
            p[1] as f32 / 128. - 0.5,
            2.,
            0.125,
        ]
    }
}
impl Source for WideRamp {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some(CellRegion::new([-1024, -32], [2048, 64])),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::Graded,
        }
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("demand-independent Gaussian ramp"))
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        self.0.lock().unwrap().push(region);
        Ok((region.y..region.bottom())
            .flat_map(|y| (region.x..region.right()).map(move |x| Self::pixel([x, y])))
            .collect())
    }
}

#[test]
fn captured_pixel_math_can_extend_its_initial_demand_without_authoring_another_value() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let plate = Plate::new();
    let initial = CellRegion::new([-4, 5], [1, 1]);
    let later = CellRegion::new([4, 9], [5, 3]);
    let input = source(Arc::new(plate.clone())).request(initial).unwrap();
    let value = Value::new(
        plate.domain,
        initial,
        vec![input],
        Operation::new(
            "extended affine demand",
            (2f64, -1f64),
            AFFINE,
            "pixel",
            Access::Pointwise,
        )
        .with_uniform("Parameters", &[2f32, -1., 0., 0.]),
    )
    .unwrap();
    let demand = value.request(later).unwrap();
    assert!(value.same_capture(&demand));
    assert_eq!(value.identity(), demand.identity());
    let (calculation, definitions) =
        testing::observe(0, || demand.lower(later, budget(later)).unwrap());
    assert_eq!(definitions, 0);
    assert_eq!(calculation.coverage(), later);
    assert_eq!(plate.reads.load(Ordering::Relaxed), 0);
    for policy in [Policy::Materialized, Policy::Composed] {
        let (pixels, _) = run(&device, &queue, &demand, policy);
        assert_eq!(pixels.len(), (later.width * later.height) as usize);
        for (index, pixel) in pixels.iter().enumerate() {
            let cell = [
                later.x + (index % later.width as usize) as i32,
                later.y + (index / later.width as usize) as i32,
            ];
            let original = plate.pixel(cell);
            let position = plate
                .domain
                .point([f64::from(cell[0]) + 0.5, f64::from(cell[1]) + 0.5]);
            for channel in 0..3 {
                let expected = original[channel] * 2. - 1. + position[0] as f32 * 0.01;
                assert!(
                    (pixel[channel] - expected).abs() < 2e-6,
                    "{cell:?}: {pixel:?} versus {expected}"
                );
            }
            assert_eq!(pixel[3].to_bits(), original[3].to_bits());
        }
    }
    assert!(plate.reads.load(Ordering::Relaxed) > 0);
}

#[test]
fn requested_values_own_their_math_and_redescribe_smaller_neighbourhoods_under_new_allowances() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Value>();
    send_sync::<crate::gpu::test_inputs::AuthoredPhotograph>();
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let reads = Arc::new(Mutex::new(Vec::new()));
    let input = source(Arc::new(WideRamp(reads.clone())));
    let group = gaussian(input.clone(), 1.).unwrap();
    let full = group.domain().finite().unwrap();
    let weak_group = group.downgrade();
    let weak_input = input.downgrade();
    let ((wide, first), descriptions) = testing::observe(0, || {
        let wide = group.request(full).unwrap();
        let first = wide.request(CellRegion::new([-96, -8], [64, 8])).unwrap();
        (wide, first)
    });
    assert_eq!(
        descriptions, 0,
        "requesting a view does not expand its calculations"
    );
    assert_eq!(wide.identity(), first.identity());
    assert!(
        wide.same_capture(&first),
        "coverage is not a second mathematical definition"
    );
    drop(group);
    drop(input);
    assert!(
        weak_group.upgrade().is_some() && weak_input.upgrade().is_some(),
        "requested values must retain their complete mathematical definitions"
    );
    assert!(reads.lock().unwrap().is_empty());

    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    for (region, allowance) in [
        (first.coverage(), 32 << 10),
        (CellRegion::new([40, 5], [48, 6]), 64 << 10),
    ] {
        let demand = wide.request(region).unwrap();
        let before = reads.lock().unwrap().len();
        let ((calculation, instructions), definitions) = testing::observe(0, || {
            testing::observe_calculations(MAX_UNIT_VALUES, || {
                wide.lower(
                    region,
                    Budget {
                        tile: [region.width, region.height],
                        scratch_bytes: allowance,
                    },
                )
                .unwrap()
            })
        });
        assert_eq!(
            definitions, 0,
            "lowering cannot author replacement mathematical Values"
        );
        assert!(
            instructions > 0,
            "the control must actually lower the spatial calculation"
        );
        assert!(calculation.primitive().is_some());
        assert_eq!(calculation.coverage(), region);
        assert_eq!(reads.lock().unwrap().len(), before);
        drop(calculation);
        let query = context
            .request(
                demand,
                Region {
                    bounds: region,
                    grid: wide.domain().grid,
                },
                Budget {
                    tile: [region.width, region.height],
                    scratch_bytes: allowance,
                },
            )
            .unwrap();
        let mut delivered = std::collections::HashSet::new();
        context
            .read(
                &queue,
                &Default::default(),
                &query,
                || false,
                |block, pixels| {
                    assert!(region.contains_region(block));
                    assert_eq!(pixels.len(), (block.width * block.height) as usize);
                    for (i, pixel) in pixels.iter().enumerate() {
                        let p = [
                            block.x + (i % block.width as usize) as i32,
                            block.y + (i / block.width as usize) as i32,
                        ];
                        assert!(delivered.insert(p), "duplicate delivery at {p:?}");
                        let expected = WideRamp::pixel(p);
                        // A symmetric unit-sum Gaussian preserves an interior affine ramp.
                        for channel in 0..3 {
                            assert!(
                                (pixel[channel] - expected[channel]).abs() < 2e-6,
                                "{p:?}: {pixel:?} versus {expected:?}"
                            );
                        }
                        assert_eq!(pixel[3].to_bits(), expected[3].to_bits());
                    }
                    Ok(())
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(delivered.len(), (region.width * region.height) as usize);
        let acquired = std::mem::take(&mut *reads.lock().unwrap());
        assert!(
            !acquired.is_empty(),
            "the cold request must actually exercise acquisition"
        );
        let support = CellRegion::new(
            [region.x - 4, region.y - 4],
            [region.width + 8, region.height + 8],
        );
        assert!(
            acquired.iter().all(|read| support.contains_region(*read)),
            "a new demand must not acquire the old full-width neighbourhood: {acquired:?}"
        );
        assert!(
            acquired
                .iter()
                .map(|r| u64::from(r.width) * u64::from(r.height))
                .sum::<u64>()
                < u64::from(full.width) * u64::from(full.height)
        );
    }
    drop((first, wide, context));
    assert!(
        weak_group.upgrade().is_none() && weak_input.upgrade().is_none(),
        "weak provenance must not retain retired calculation definitions"
    );
}
