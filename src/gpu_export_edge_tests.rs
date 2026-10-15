use super::*;
use crate::{
    geometry::{PhotoOrientation, RollOrientation, SourcePoint},
    local_adjustments::{GradientMask, LocalAdjustment, SpatialPipeline},
    processing::PhotoSettings,
};

#[test]
fn small_edge_tiles_keep_precise_coordinates_unpadded_readback_and_bounded_storage() {
    let (device, queue, renderer) = super::tests::context();
    let submissions = SubmissionGate::default();
    let area = SourceRegion::from([(1 << 24) - 17, (1 << 24) - 9, 17, 9]);
    let calibration = Calibration {
        dmin: DVec3::splat(0.2),
        dmax: DVec3::splat(0.02),
        input_floor: 1. / 65535.,
    };
    let roll = RollSettings::new_grade();
    let snapshot = PhotoGradeSnapshot::new(
        PhotoSettings::default(),
        vec![LocalAdjustment {
            mask: GradientMask::Linear {
                start: SourcePoint([area.x as f64, area.y as f64]),
                end: SourcePoint([(area.x + area.width) as f64, area.y as f64]),
            },
            photo: PhotoSettings {
                exposure: 0.5,
                ..Default::default()
            },
            enabled: true,
            invert: false,
            strength: 0.8,
        }],
    )
    .unwrap();
    let scalar =
        SpatialPipeline::new(calibration, roll, snapshot.clone(), DMat3::IDENTITY).unwrap();
    for format in [
        Format::Exr(ExrPrecision::Full),
        Format::Tiff(OutputTarget::Srgb),
    ] {
        let grade = GpuPhotoGrade::new(
            &device,
            calibration,
            roll,
            snapshot.clone(),
            DMat3::IDENTITY,
            area,
        )
        .unwrap();
        let mut processor = OriginalTiles::new(
            &renderer,
            (&device, &queue, &submissions),
            (DMat3::IDENTITY, area),
            None,
            grade,
            format,
        )
        .unwrap();
        assert_eq!(processor.size, [17, 9]);
        assert_eq!(processor.source.byte_len(), 17 * 9 * 8);
        let maximum = u64::from(TILE[0]) * u64::from(TILE[1]) * 48;
        assert!(processor.working.size() <= maximum);
        assert!(processor.readback.size() <= maximum);
        let pixels: Vec<_> = (0..17 * 9)
            .map(|i| {
                [
                    1000 + i as u16 * 100,
                    4000 + i as u16 * 50,
                    16000 - i as u16 * 30,
                    0,
                ]
            })
            .collect();
        let actual = processor
            .read(
                &renderer,
                &device,
                &queue,
                &submissions,
                [area.x, area.y],
                &pixels,
            )
            .unwrap();
        assert_eq!(actual.len(), pixels.len());
        assert!(actual.iter().flatten().all(|v| v.is_finite()));
        if let Format::Exr(_) = format {
            for (i, (actual, pixel)) in actual.iter().zip(&pixels).enumerate() {
                let point = [area.x + i as u32 % 17, area.y + i as u32 / 17];
                let expected = scalar
                    .apply(
                        DVec3::new(pixel[0] as f64, pixel[1] as f64, pixel[2] as f64) / 65535.,
                        point,
                    )
                    .unwrap()
                    .rgb;
                for c in 0..3 {
                    assert!((actual[c] - expected[c]).abs() <= 2e-12 + 2e-11 * expected[c].abs());
                }
            }
        } else {
            assert!(
                actual
                    .iter()
                    .all(|p| p[..3].iter().all(|v| (0. ..=1.).contains(v)))
            );
        }
    }
    let composition = Composition::new(
        area,
        RollOrientation::default(),
        PhotoOrientation::default(),
    )
    .unwrap();
    let result = reconstruct(composition, [0, 0], [17, 9], |origin| {
        assert_eq!(origin, [area.x, area.y]);
        Ok(vec![[1., 2., 3., 0.]; 17 * 9])
    })
    .unwrap();
    assert!(result.iter().all(|p| *p == DVec3::new(1., 2., 3.)));
}
