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
    let mut excluded = 0;
    let mut included = 0;
    for origin in [[0, 0], [384, 128], [128, 384], [0, 384]] {
        let unit = Unit::new(composition, origin, [128; 2]).unwrap();
        assert_eq!(unit.mapping.space[1], 1);
        let bits = unit.support.as_ref().unwrap();
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
    assert!(interior.support.is_none());
    assert_eq!(interior.mapping.space[1], 0);
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
