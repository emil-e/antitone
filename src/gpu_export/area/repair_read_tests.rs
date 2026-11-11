use crate::gpu::evaluation as e;
use std::sync::Arc;

#[test]
fn masked_replacement_declares_only_its_requested_original_cells_without_reading_pixels() {
    struct Unread(e::Domain);
    impl e::Source for Unread {
        fn domain(&self) -> e::Domain {
            self.0
        }
        fn read(&self, _: e::CellRegion) -> Result<Vec<[f32; 4]>, crate::Error> {
            panic!("describing reads must not acquire pixels")
        }
    }
    let domain = e::Domain {
        bounds: Some(e::CellRegion::new([0, 0], [128, 128])),
        grid: e::Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: e::Meaning::Neutral,
    };
    let base = e::source(Arc::new(Unread(domain)));
    let replacement = e::source(Arc::new(Unread(e::Domain {
        bounds: Some(e::CellRegion::new([16, 24], [16, 16])),
        ..domain
    })));
    let support =
        crate::dust_repair::SupportMask::new([16, 24], [16, 16], vec![true; 256]).unwrap();
    let root = crate::gpu::repairs::definition::compose_images(
        base.clone(),
        vec![(support, replacement.clone())],
    )
    .unwrap();
    for (output, expected) in [
        (
            e::CellRegion::new([0, 0], [128, 128]),
            vec![e::CellRegion::new([16, 24], [16, 16])],
        ),
        (
            e::CellRegion::new([20, 28], [4, 6]),
            vec![e::CellRegion::new([20, 28], [4, 6])],
        ),
        (e::CellRegion::new([100, 100], [8, 8]), Vec::new()),
    ] {
        assert_eq!(
            root.regions_for(
                &replacement,
                output,
                e::Budget {
                    tile: [128; 2],
                    scratch_bytes: 1 << 20
                }
            )
            .unwrap(),
            expected
        );
        assert_eq!(
            root.regions_for(
                &base,
                output,
                e::Budget {
                    tile: [128; 2],
                    scratch_bytes: 1 << 20
                }
            )
            .unwrap(),
            vec![output]
        );
    }
}
