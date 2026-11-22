use super::*;

struct Unreadable;
impl Source for Unreadable {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some(Rect::new([0, 0], [8192, 6144])),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        }
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("large unreadable planning witness"))
    }
    fn read(&self, _: Rect) -> Result<Vec<[f32; 4]>, Error> {
        panic!("description must not read original pixels")
    }
}

struct WideUnreadable;
impl Source for WideUnreadable {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some(Rect::new([0, 0], [1_000_000; 2])),
            ..Unreadable.domain()
        }
    }
    fn read(&self, _: Rect) -> Result<Vec<[f32; 4]>, Error> {
        panic!("description must not read original pixels")
    }
}

#[test]
fn a_bounded_rectangular_contribution_checks_device_storage_before_source_io() {
    let (input, reads) = fixture([512, 512], vec![[0.; 4]; 512 * 512]);
    let map = Mapping {
        start: [0.; 2],
        axes: [[8., 0.], [0., 8.]],
    };
    let region = Rect::new([0, 0], [64, 64]);
    let root = reconstruct_mapped(input, output(map, region), map).unwrap();
    let budget = Tiling { tile: [64, 64] };
    let mut unit = root.plan_unit(region, budget).unwrap();
    let UnitWork::Gpu(work) = &mut unit.work else {
        panic!("expected a GPU calculation")
    };
    let phases = work.phases.by_ref().collect::<Result<Vec<_>, _>>().unwrap();
    let source_reads = phases
        .iter()
        .flat_map(|phase| &phase.reads)
        .map(|read| read.region)
        .collect::<Vec<_>>();
    assert_eq!(
        source_reads,
        [Rect::new([0, 0], [512, 512])],
        "a small-footprint rectangular contribution may coalesce its source window"
    );
    let limits = wgpu::Limits {
        max_storage_buffer_binding_size: 1 << 20,
        ..wgpu::Limits::default()
    };
    assert!(
        work.state
            .iter()
            .all(|bytes| *bytes <= limits.max_storage_buffer_binding_size)
    );
    assert!(
        u64::from(region.width) * u64::from(region.height) * 16
            <= limits.max_storage_buffer_binding_size
    );
    assert!(matches!(
        root.plan_unit(region, budget).unwrap().admit_for(&limits),
        Err(Error::EvaluationLimit(
            "GPU unit exceeds device storage limits"
        ))
    ));
    assert!(reads.lock().unwrap().is_empty());
}

#[test]
fn a_large_request_captures_output_without_expanding_calculations_under_its_full_extent() {
    let input = source(Arc::new(Unreadable));
    let map = Mapping {
        start: [0.; 2],
        axes: [[8., 0.], [0., 6.]],
    };
    let region = Rect::new([0, 0], [1024, 1024]);
    let domain = output(map, region);
    let root = reconstruct_mapped(input, domain, map).unwrap();
    let root = crate::gpu::gaussian::blur(root, 0.1).unwrap();
    let ((job, described), created) = value::testing::observe(0, || {
        (
            super::super::super::super::plan::Job::new(
                root.clone(),
                region,
                Tiling { tile: [1024; 2] },
            )
            .unwrap(),
            root.output(region).unwrap(),
        )
    });
    assert_eq!(
        created, 0,
        "request metadata must not expand even an ordinary photographic group"
    );
    assert_eq!(job.region(), region);
    assert_eq!(described.domain, domain);
    assert_eq!(described.region, region);
    assert!(described.identity().is_some());
    assert_eq!(described.identity(), job.image().identity());
}

#[test]
fn large_aperture_describes_normal_contributions_without_a_memory_allowance_or_source_io() {
    let input = source(Arc::new(Unreadable));
    let map = Mapping {
        start: [0.; 2],
        axes: [[8192., 0.], [0., 6144.]],
    };
    let region = Rect::new([0, 0], [1, 1]);
    let domain = output(map, region);
    let root = reconstruct_mapped(input, domain, map).unwrap();
    let (result, created) = crate::gpu::evaluation::value::testing::observe_calculations(
        value::MAX_UNIT_VALUES + 4,
        || root.plan_unit(region, Tiling { tile: [1, 1] }),
    );
    let unit = result.unwrap();
    assert!(matches!(unit.work, UnitWork::Gpu(_)));
    assert!(
        created > 4 && created <= value::MAX_UNIT_VALUES,
        "source geometry produces bounded descriptions, not a working-byte rejection"
    );
}

#[test]
fn thin_rotated_footprints_bound_scanning_of_noncontributing_source_partitions() {
    let input = source(Arc::new(WideUnreadable));
    let map = Mapping {
        start: [0.; 2],
        axes: [[131_072., 100_000.], [131_072., 100_001.]],
    };
    let region = Rect::new([0, 0], [1, 1]);
    let domain = output(map, region);
    let root = reconstruct_mapped(input, domain, map).unwrap();
    let (result, _) = value::testing::observe_calculations(value::MAX_UNIT_VALUES + 4, || {
        root.plan_unit(region, Tiling { tile: [1, 1] })
    });
    assert!(
        matches!(
            result,
            Err(Error::EvaluationLimit(
                "calculation exceeds the unit graph capacity"
            ))
        ),
        "empty partitions also consume bounded planning work"
    );
}

#[test]
fn large_source_domains_have_bounded_unit_description_storage() {
    let input = source(Arc::new(WideUnreadable));
    let map = Mapping {
        start: [0.; 2],
        axes: [[131_072., 0.], [0., 131_072.]],
    };
    let region = Rect::new([0, 0], [1, 1]);
    let domain = output(map, region);
    let root = reconstruct_mapped(input, domain, map).unwrap();
    let (result, created) = crate::gpu::evaluation::value::testing::observe_calculations(
        value::MAX_UNIT_VALUES + 4,
        || root.plan_unit(region, Tiling { tile: [1, 1] }),
    );
    assert!(matches!(
        result,
        Err(Error::EvaluationLimit(
            "calculation exceeds the unit graph capacity"
        ))
    ));
    assert!(
        created <= value::MAX_UNIT_VALUES,
        "host descriptions must stay bounded independently of source dimensions"
    );
}
