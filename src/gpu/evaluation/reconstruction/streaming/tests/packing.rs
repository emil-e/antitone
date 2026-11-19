use super::*;

struct PlanningSource;

impl Source for PlanningSource {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some(Rect::new([0, 0], [1024, 1024])),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::Graded,
        }
    }

    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("oblique read packing witness"))
    }

    fn read(&self, _: Rect) -> Result<Vec<[f32; 4]>, Error> {
        panic!("planning must not acquire source pixels")
    }
}

#[test]
fn oblique_reconstruction_bounds_read_count_by_source_tiles_instead_of_edge_steps() {
    for degrees in [-3_f64, 3.] {
        let (sin, cos) = degrees.to_radians().sin_cos();
        let map = Mapping {
            start: [128., 128.],
            axes: [[640. * cos, 640. * sin], [-640. * sin, 640. * cos]],
        };
        let region = Rect::new([0, 0], [1, 1]);
        let root =
            reconstruct_mapped(source(Arc::new(PlanningSource)), output(map, region), map).unwrap();
        let mut unit = root.plan_unit(region, Tiling { tile: [1, 1] }).unwrap();
        let UnitWork::Gpu(work) = &mut unit.work else {
            panic!("expected a GPU reconstruction");
        };
        let phases = work.phases.by_ref().collect::<Result<Vec<_>, _>>().unwrap();
        let reads = phases
            .iter()
            .flat_map(|phase| &phase.reads)
            .map(|read| read.region)
            .collect::<Vec<_>>();

        assert!(reads.len() > 1, "exercise bounded source partitioning");
        // The ~673-cell-wide footprint fits within two 512-wide columns and
        // eight 128-high rows, independently of its many sloping edge steps.
        assert!(reads.len() <= 16, "degrees={degrees} reads={reads:?}");
        assert!(
            reads
                .iter()
                .all(|read| read.width <= 512 && read.height <= 128),
            "degrees={degrees} reads={reads:?}"
        );
    }
}

#[test]
fn an_enclosing_source_read_does_not_give_unused_diamond_corners_area_or_flags() {
    let size = [128, 128];
    let mut values = vec![[-2., 3., 5., 1.]; 128 * 128];
    for y in 0..8 {
        for x in 0..8 {
            for (x, y) in [(x, y), (127 - x, y), (x, 127 - y), (127 - x, 127 - y)] {
                values[y * 128 + x] = [9999., 9999., 9999., 4.];
            }
        }
    }
    let (input, reads) = fixture(size, values);
    let map = Mapping {
        start: [64., 0.],
        axes: [[64., 64.], [-64., 64.]],
    };
    let actual = materialize(
        reconstruct_mapped(input, output(map, [0, 0, 1, 1].into()), map).unwrap(),
        [1, 1],
    );

    assert_eq!(
        *reads.lock().unwrap(),
        [Rect::new([0, 0], size)],
        "one admitted window intentionally acquires the unused corners"
    );
    assert_eq!(actual[0][3], 9., "unused corner flags must not contribute");
    for (actual, expected) in actual[0][..3].iter().zip([-2., 3., 5.]) {
        assert!((actual - expected).abs() < 2e-5, "{actual:?}");
    }
}
