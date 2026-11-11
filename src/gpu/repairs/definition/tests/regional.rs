use super::*;
use crate::gpu::evaluation::reconstruct_tiled as reconstruct;
mod quantum;
mod retention;
use crate::dust_repair::{RepairId, RepairOrigin};
use std::sync::Mutex;

struct ObservedPlate(Arc<Mutex<Vec<CellRegion>>>);
impl Source for ObservedPlate {
    fn domain(&self) -> Domain {
        Plate.domain()
    }
    fn identity(&self) -> Option<RetainedKey> {
        Plate.identity()
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        self.0.lock().unwrap().push(region);
        Plate.read(region)
    }
}

struct Plate;
impl Source for Plate {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some([0, 0, 512, 256].into()),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::Neutral,
        }
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("regional signed plate"))
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        Ok((region.y..region.bottom())
            .flat_map(|y| {
                (region.x..region.right())
                    .map(move |x| [1. + x as f32 / 512., -0.5 - y as f32 / 256., 0.25, 0.75])
            })
            .collect())
    }
}
fn repair(x: u32, offset: i32) -> Repair {
    Repair {
        id: RepairId(u64::from(x)),
        enabled: true,
        origin: RepairOrigin::Manual,
        support: SupportMask::new(
            [x, 40],
            [3, 3],
            vec![false, false, false, false, true, false, false, false, false],
        )
        .unwrap(),
        offset: [offset, 32],
    }
}
fn view(repairs: &[Repair]) -> Value {
    let base = source(Arc::new(Plate));
    let domain = Domain {
        bounds: Some([0, 0, 256, 128].into()),
        grid: Grid::axis_aligned([0.; 2], [2.; 2]),
        meaning: Meaning::Neutral,
    };
    reconstruct(compose(base, repairs).unwrap(), domain).unwrap()
}
fn query(context: &Context, root: Value) -> Evaluation {
    let domain = root.domain();
    context
        .request(
            root,
            Region {
                bounds: domain.finite().unwrap(),
                grid: domain.grid,
            },
            Budget {
                tile: [256, 128],
                // Keep multiple natural units in this small reuse fixture.
                scratch_bytes: 2 << 20,
            },
        )
        .unwrap()
}
#[test]
fn regional_descriptor_attachment_converges_for_named_and_anonymous_inputs() {
    struct Anonymous;
    impl Source for Anonymous {
        fn domain(&self) -> Domain {
            Plate.domain()
        }
        fn read(&self, _: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
            panic!("description attachment must not acquire pixels")
        }
    }
    for named in [true, false] {
        let input: Arc<dyn Source> = if named {
            Arc::new(Plate)
        } else {
            Arc::new(Anonymous)
        };
        let input = source(input);
        let image = reconstruct(
            compose(input, &[repair(40, 32), repair(360, 32)]).unwrap(),
            view(&[]).domain(),
        )
        .unwrap();
        let region = CellRegion::new([0, 0], [128, 128]);
        let (_, created) = value::testing::observe(0, || {
            let requested = image.request(region).unwrap();
            let again = requested.request(region).unwrap();
            assert!(
                requested.same_capture(&again),
                "equivalent regional metadata must not look like another calculation expansion"
            );
            assert!(
                requested
                    .output(region)
                    .unwrap()
                    .same_value(&image.output(region).unwrap())
            );
            assert_eq!(requested.identity().is_some(), named);
        });
        assert_eq!(created, 0);
    }
}

#[test]
fn output_descriptions_do_not_expand_calculations_and_coverage_is_not_identity() {
    let node = source(Arc::new(Plate));
    let ((left, right), count) = value::testing::observe(0, || {
        (
            node.output([0, 0, 128, 128].into()).unwrap(),
            node.output([128, 0, 128, 128].into()).unwrap(),
        )
    });
    assert_eq!(count, 0);
    assert_eq!(left.domain, node.domain());
    assert_ne!(left.region, right.region);
    assert!(left.identity().is_some());
    assert_eq!(left.identity(), right.identity());
}
#[test]
fn only_influencing_repair_instructions_change_a_regional_output() {
    let a = repair(40, 32);
    let b = repair(360, 32);
    let original = view(std::slice::from_ref(&a));
    let added = view(&[a.clone(), b.clone()]);
    let left = [0, 0, 128, 128].into();
    let right = [128, 0, 128, 128].into();
    assert!(
        !original.same_definition(&added),
        "full intent must notice the added repair"
    );
    let changed = view(&[repair(40, 48), b]);
    let removed = view(&[a]);
    let (_, count) = value::testing::observe(0, || {
        assert_eq!(original.output(left).unwrap(), added.output(left).unwrap());
        assert_ne!(
            original.output(right).unwrap(),
            added.output(right).unwrap()
        );
        assert_ne!(
            added.output(left).unwrap(),
            changed.output(left).unwrap(),
            "a changed donor must invalidate the target region"
        );
        assert_eq!(added.output(right).unwrap(), changed.output(right).unwrap());
        assert_ne!(
            added.output(right).unwrap(),
            removed.output(right).unwrap(),
            "removal restores original cells rather than keeping stale repair pixels"
        );
    });
    assert_eq!(
        count, 0,
        "regional output description must not rebuild a calculation"
    );
}
#[test]
fn a_new_repair_reuses_the_actual_gpu_value_for_an_unchanged_view_region() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 32 << 20,
        },
    );
    let scope = context.materializations();
    scope.set_test_pressure(crate::sys::MemoryPressure::Normal);
    let reads = Arc::new(Mutex::new(Vec::new()));
    let make = |repairs: &[Repair]| {
        let base = source(Arc::new(ObservedPlate(reads.clone())));
        reconstruct(compose(base, repairs).unwrap(), view(repairs).domain()).unwrap()
    };
    let original = make(&[repair(40, 32)]);
    let changed = make(&[repair(40, 32), repair(360, 32)]);
    let region: CellRegion = [0, 0, 128, 128].into();
    let output = original.output(region).unwrap();
    let held = pollster::block_on(
        context
            .materialize(
                &queue,
                &Default::default(),
                &scope,
                query(&context, original),
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap();
    let before = scope
        .find_ready::<ImageLayout>(output.identity().unwrap(), |layout| layout.region == region)
        .expect("the regional GPU result must actually have been produced");
    assert!(
        reads.lock().unwrap().iter().any(|r| r.x < 256),
        "the cold calculation must actually acquire unchanged-region inputs"
    );
    reads.lock().unwrap().clear();
    let updated = pollster::block_on(
        context
            .materialize(
                &queue,
                &Default::default(),
                &scope,
                query(&context, changed.clone()),
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap();
    let after = scope
        .find_ready::<ImageLayout>(
            changed.output(region).unwrap().identity().unwrap(),
            |layout| layout.region == region,
        )
        .unwrap();
    assert!(
        Arc::ptr_eq(&before.image, &after.image),
        "reuse must mean the physical GPU value, not merely equivalent definitions"
    );
    let acquired = reads.lock().unwrap();
    assert!(
        !acquired.is_empty(),
        "the changed region must actually execute, not leave the old full frame in place"
    );
    assert!(
        acquired.iter().all(|r| r.x >= 256),
        "the new frame must consume the retained left tile instead of recomputing it: {acquired:?}"
    );
    drop((held, updated));
}

#[test]
fn regional_and_direct_reconstruction_preserve_ramps_across_tiles_with_no_optional_retention() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let base = source(Arc::new(Plate));
    let repaired = compose(base, &[repair(254, 32)]).unwrap();
    let domain = Domain {
        bounds: Some([0, 0, 256, 128].into()),
        grid: Grid::axis_aligned([0.; 2], [2.; 2]),
        meaning: Meaning::Neutral,
    };
    let regional = reconstruct(repaired.clone(), domain).unwrap();
    let direct = crate::gpu::evaluation::reconstruct(repaired, domain).unwrap();
    let read = |node| {
        let mut pixels = vec![[0.; 4]; 256 * 128];
        context
            .read(
                &queue,
                &Default::default(),
                &query(&context, node),
                || false,
                |block, values| {
                    for row in 0..block.height {
                        let from = row as usize * block.width as usize;
                        let to = domain
                            .finite()
                            .unwrap()
                            .index([block.x, block.y + row as i32]);
                        pixels[to..to + block.width as usize]
                            .copy_from_slice(&values[from..from + block.width as usize]);
                    }
                    Ok(())
                },
            )
            .unwrap()
            .unwrap();
        pixels
    };
    let expected = read(direct);
    let actual = read(regional);
    for (a, b) in actual.iter().zip(expected) {
        for (x, y) in a.iter().zip(b) {
            assert!(
                (x - y).abs() < 2e-6,
                "partitioning must not change sampling or metadata"
            );
        }
    }
    // Exact area averages of the independently authored source ramp, away from repair.
    assert_eq!(actual[0], [1. + 0.5 / 512., -0.5 - 0.5 / 256., 0.25, 0.]);
    assert_ne!(
        actual[0], actual[255],
        "coordinate controls must detect region substitutions or tile-local addressing"
    );
}

#[test]
fn partial_tiles_do_not_run_a_repair_outside_the_requested_pixels() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let root = view(&[repair(100, 10000)]);
    let read = |bounds| {
        let query = context
            .request(
                root.clone(),
                Region {
                    bounds,
                    grid: root.domain().grid,
                },
                Budget {
                    tile: [128, 128],
                    scratch_bytes: 16 << 20,
                },
            )
            .unwrap();
        context.read(&queue, &Default::default(), &query, || false, |_, _| Ok(()))
    };
    assert!(
        read([0, 0, 32, 32].into()).unwrap().is_some(),
        "cache tiling must not widen demand to the invalid donor in the rest of the tile"
    );
    assert!(
        read([48, 18, 8, 8].into()).is_err(),
        "the control must actually require the invalid donor"
    );
}

#[test]
fn regional_units_fit_small_allowances_without_changing_demand() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let root = view(&[repair(40, 32)]);
    // Leave working storage beyond the output itself for upstream reconstruction.
    let region: CellRegion = [0, 0, 32, 32].into();
    let budget = Budget {
        tile: [32, 32],
        scratch_bytes: 64 << 10,
    };
    let units = root
        .operation()
        .work_units(region, budget)
        .unwrap()
        .collect::<Vec<_>>();
    assert!(units.iter().all(|unit| region.contains_region(*unit)));
    assert_eq!(units.iter().map(|u| u.width * u.height).sum::<u32>(), 1024);
    let query = context
        .request(
            root.clone(),
            Region {
                bounds: region,
                grid: root.domain().grid,
            },
            budget,
        )
        .unwrap();
    let mut cells = 0;
    context
        .read(
            &queue,
            &Default::default(),
            &query,
            || false,
            |_, pixels| {
                cells += pixels.len();
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        cells, 1024,
        "admission must preserve the complete output request"
    );
}

#[test]
fn unbounded_view_regions_with_negative_origins_keep_real_source_edges() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let repaired = compose(source(Arc::new(Plate)), &[repair(40, 32)]).unwrap();
    let domain = Domain {
        bounds: None,
        grid: Grid::axis_aligned([0.; 2], [2.; 2]),
        meaning: Meaning::Neutral,
    };
    let tiled = reconstruct(repaired.clone(), domain).unwrap();
    let direct = crate::gpu::evaluation::reconstruct(repaired, domain).unwrap();
    let region: CellRegion = [-7, -5, 151, 83].into();
    let read = |root| {
        let evaluation = context
            .request(
                root,
                Region {
                    bounds: region,
                    grid: domain.grid,
                },
                Budget {
                    tile: [151, 83],
                    scratch_bytes: 16 << 20,
                },
            )
            .unwrap();
        let mut values = vec![[0.; 4]; 151 * 83];
        context
            .read(
                &queue,
                &Default::default(),
                &evaluation,
                || false,
                |part, pixels| {
                    for row in 0..part.height {
                        let from = row as usize * part.width as usize;
                        let to = region.index([part.x, part.y + row as i32]);
                        values[to..to + part.width as usize]
                            .copy_from_slice(&pixels[from..from + part.width as usize]);
                    }
                    Ok(())
                },
            )
            .unwrap()
            .unwrap();
        values
    };
    let expected = read(direct);
    let actual = read(tiled);
    assert_eq!(
        actual[0],
        [1., -0.5, 0.25, 0.],
        "only real source bounds extend constantly"
    );
    for (a, b) in actual.iter().zip(expected) {
        for (x, y) in a.iter().zip(b) {
            assert!((x - y).abs() < 2e-6);
        }
    }
}
