use super::*;

#[test]
fn edge_masks_share_exact_whole_frame_relevance_with_repair_preparation() {
    let composition = Composition::with_crop(
        [0, 0, 1024, 1024].into(),
        Default::default(),
        Default::default(),
        Some(crate::crop::Crop {
            center: [512.; 2],
            size: [512; 2],
            angle: 0.13,
        }),
    )
    .unwrap();
    let coverage = composition
        .coverage(FramePoint([0.; 2]), FramePoint([512.; 2]))
        .unwrap();
    let input = crate::gpu::evaluation::Domain {
        bounds: Some(composition.area().try_into().unwrap()),
        grid: crate::gpu::evaluation::Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: crate::gpu::evaluation::Meaning::Graded,
    };
    let frame = FrameRaster::full(composition).unwrap();
    let root = crate::gpu::evaluation::reconstruct_complete(
        crate::gpu::evaluation::borrowed_input(input),
        crate::gpu::evaluation::Domain {
            bounds: Some(crate::gpu::evaluation::CellRegion::new(
                [0, 0],
                frame.output,
            )),
            grid: crate::gpu::evaluation::Grid::from_frame(frame),
            meaning: input.meaning,
        },
    )
    .unwrap();
    let mut excluded = 0;
    let mut included = 0;
    for origin in [[0, 0], [384, 128], [128, 384], [0, 384]] {
        let unit = Unit::new(composition, origin, [128; 2]).unwrap();
        let bits = crate::gpu::evaluation::reconstruction_mask_for_test(
            &root,
            [origin[0], origin[1], 128, 128].try_into().unwrap(),
            unit.bounds.try_into().unwrap(),
        )
        .unwrap();
        for y in 0..unit.bounds.height {
            for x in 0..unit.bounds.width {
                let index = y * unit.bounds.width + x;
                let admitted = bits[index as usize / 32] & (1 << (index % 32)) != 0;
                let relevant = coverage.weight([unit.bounds.x + x, unit.bounds.y + y]) > 0.;
                assert_eq!(admitted, relevant);
                if admitted {
                    included += 1;
                } else {
                    excluded += 1;
                }
            }
        }
    }
    assert!(
        included > 0 && excluded > 0,
        "both mask branches must be exercised"
    );
    let interior = Unit::new(composition, [128; 2], [128; 2]).unwrap();
    assert!(
        crate::gpu::evaluation::reconstruction_mask_for_test(
            &root,
            [128, 128, 128, 128].into(),
            interior.bounds.try_into().unwrap()
        )
        .is_none()
    );
}

#[test]
fn native_roundoff_cannot_admit_invalid_cells_outside_exact_frame_in_either_linear_space() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let engine = Reconstructor::new(&device).unwrap();
    let pixel = |p| {
        if p == [1, 1] {
            [0.25, 0.5, 2., 0.]
        } else {
            [f32::NAN, 0., 0., -1.]
        }
    };
    for space in [0, 1] {
        let mut unit = Unit {
            bounds: [0, 0, 3, 3].into(),
            mapping: Mapping {
                sizes: [3, 3, 1, 1],
                // Isolate an exactly declared cell from an emulated native-map edge error.
                origin: [1. - 2_f32.powi(-20), 1., 0., 0.],
                dx: [1., 0., 0., 0.],
                dy: [0., 1., 0., 0.],
                space: [space, 1, 0, 0],
            },
            support: Some(vec![1 << 4]),
        };
        let result = super::tests::reconstructed(&device, &queue, &engine, &unit, pixel, None)[0];
        assert_eq!(&result[..3], &[0.25, 0.5, 2.]);
        assert!(result[3] > 0.);
        unit.mapping.space[1] = 0;
        unit.support = None;
        let unmasked = super::tests::reconstructed(&device, &queue, &engine, &unit, pixel, None)[0];
        assert!(
            unmasked[3] < 0.,
            "control must actually consume the invalid sliver, space={space}"
        );
    }
}
