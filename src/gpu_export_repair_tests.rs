use super::*;
use crate::{
    color::{Balance, wb_matrix},
    dust_repair::{DonorChoice, Repair, RepairId, RepairOrigin, SupportMask},
    geometry::{SourcePoint, SourceRegion},
    local_adjustments::{GradientMask, LocalAdjustment, LocalGrade},
    processing::{InvertedPixel, PhotoSettings, exposure_multiplier},
};

fn patch(
    device: &wgpu::Device,
    origin: [u32; 2],
    active: SourceRegion,
) -> crate::gpu::repairs::Snapshot {
    let repair = Repair {
        id: RepairId(1),
        enabled: true,
        origin: RepairOrigin::Manual,
        support: SupportMask::new(
            origin,
            [3, 3],
            vec![true, false, true, false, true, false, true, false, true],
        )
        .unwrap(),
        donor: DonorChoice::Offset([-64, 0]),
    };
    let evaluated = crate::dust_repair::evaluate(&repair, active, |[x, y]| {
        Ok([-0.125 + x as f32 * 0.0001, 2. + y as f32 * 0.001, 0.25])
    })
    .unwrap();
    crate::gpu::repairs::Snapshot::from_evaluations(device, vec![Arc::new(evaluated)]).unwrap()
}

#[test]
fn logical_export_repairs_cross_original_units_without_narrowing_large_integer_origins() {
    let (device, queue, renderer) = tests::context();
    let gate = SubmissionGate::default();
    let start = 1 << 25;
    let area: SourceRegion = [start, start, 256, 128].into();
    let calibration = Calibration {
        dmin: DVec3::splat(0.8),
        dmax: DVec3::splat(0.01),
        input_floor: 1. / 65535.,
    };
    let roll = RollSettings::new_grade();
    let snapshot = PhotoGradeSnapshot::new(PhotoSettings::default(), vec![]).unwrap();
    let resident = patch(&device, [start + 126, start + 32], area);
    let repairs = crate::gpu::repairs::Snapshot::logical(
        resident
            .0
            .iter()
            .map(|patch| patch.evaluated.clone())
            .collect(),
    )
    .unwrap();
    assert_eq!(
        resident, repairs,
        "logical content does not depend on GPU allocation or residency"
    );
    let grade = || {
        GpuPhotoGrade::new(
            &device,
            calibration,
            roll,
            snapshot.clone(),
            DMat3::IDENTITY,
            area,
        )
        .unwrap()
    };
    let mut plain = OriginalTiles::new(
        &renderer,
        (&device, &queue, &gate),
        (DMat3::IDENTITY, area),
        None,
        grade(),
        Format::Exr(ExrPrecision::Full),
    )
    .unwrap();
    let mut repaired = OriginalTiles::new(
        &renderer,
        (&device, &queue, &gate),
        (DMat3::IDENTITY, area),
        None,
        grade(),
        Format::Exr(ExrPrecision::Full),
    )
    .unwrap();
    repaired.set_repairs(&device, repairs.clone()).unwrap();
    let mut replaced = 0;
    for origin in [[start, start], [start + 128, start]] {
        let pixels = tests::tile(origin, TILE);
        let base = plain
            .read(&renderer, &device, &queue, &gate, origin, &pixels)
            .unwrap();
        let actual = repaired
            .read(&renderer, &device, &queue, &gate, origin, &pixels)
            .unwrap();
        for (index, (base, actual)) in base.iter().zip(actual).enumerate() {
            let point = [
                origin[0] + index as u32 % TILE[0],
                origin[1] + index as u32 / TILE[0],
            ];
            if repairs.0[0].evaluated.support().contains(point) {
                replaced += 1;
                let expected = repairs.0[0].evaluated.pixel(point, [0.; 3]).map(f64::from);
                assert_eq!(&actual[..3], &expected);
                assert_eq!(actual[3], base[3]);
            } else {
                assert_eq!(actual.map(f64::to_bits), base.map(f64::to_bits));
            }
        }
    }
    assert_eq!(replaced, repairs.0[0].evaluated.support().marked_pixels());
}

#[test]
fn export_repairs_preserve_compensation_flags_untouched_words_and_independent_pair_coordinates() {
    let (device, queue, renderer) = tests::context();
    let gate = SubmissionGate::default();
    let area: SourceRegion = [10, 20, 512, 256].into();
    let calibration = Calibration {
        dmin: DVec3::splat(0.8),
        dmax: DVec3::splat(0.01),
        input_floor: 1. / 65535.,
    };
    let roll = RollSettings {
        exposure: 1.1,
        wb: Balance {
            warmth: 0.02,
            tint: -0.01,
        },
        ..RollSettings::new_grade()
    };
    let photo = PhotoSettings {
        exposure: -0.2,
        contrast: 1.2,
        saturation: 0.8,
        ..Default::default()
    };
    let local = LocalAdjustment {
        mask: GradientMask::Linear {
            start: SourcePoint([10., 20.]),
            end: SourcePoint([522., 276.]),
        },
        photo: PhotoSettings {
            exposure: 1.2,
            saturation: 1.1,
            ..Default::default()
        },
        enabled: true,
        invert: false,
        strength: 0.9,
    };
    let snapshot = PhotoGradeSnapshot::new(photo, vec![local]).unwrap();
    let locals = LocalGrade::new(snapshot.locals().to_vec()).unwrap();
    let first = [10, 20];
    let second = [266, 148];
    let mut repairs = patch(&device, [136, 50], area);
    repairs.0.extend(patch(&device, [270, 150], area).0);
    let repairs = crate::gpu::repairs::Snapshot::logical(
        repairs
            .0
            .iter()
            .map(|patch| patch.evaluated.clone())
            .collect(),
    )
    .unwrap();
    let global_matrix = wb_matrix(photo.wb).unwrap()
        * wb_matrix(roll.wb).unwrap()
        * (exposure_multiplier(roll.exposure).unwrap()
            * exposure_multiplier(photo.exposure).unwrap());
    for format in [
        Format::Exr(ExrPrecision::Full),
        Format::Tiff(OutputTarget::Srgb),
        Format::Tiff(OutputTarget::DisplayP3),
    ] {
        let grade = || {
            GpuPhotoGrade::new(
                &device,
                calibration,
                roll,
                snapshot.clone(),
                DMat3::IDENTITY,
                area,
            )
            .unwrap()
        };
        let mut plain = OriginalTiles::new(
            &renderer,
            (&device, &queue, &gate),
            (DMat3::IDENTITY, area),
            None,
            grade(),
            format,
        )
        .unwrap();
        let mut repaired = OriginalTiles::new(
            &renderer,
            (&device, &queue, &gate),
            (DMat3::IDENTITY, area),
            None,
            grade(),
            format,
        )
        .unwrap();
        repaired.set_repairs(&device, repairs.clone()).unwrap();
        let mut singles = Vec::new();
        for origin in [first, second] {
            let pixels = tests::tile(origin, TILE);
            let base = plain
                .read(&renderer, &device, &queue, &gate, origin, &pixels)
                .unwrap();
            let actual = repaired
                .read(&renderer, &device, &queue, &gate, origin, &pixels)
                .unwrap();
            for (index, (base, actual)) in base.iter().zip(&actual).enumerate() {
                let point = [
                    origin[0] + index as u32 % TILE[0],
                    origin[1] + index as u32 / TILE[0],
                ];
                let value = repairs
                    .0
                    .iter()
                    .find(|patch| patch.evaluated.support().contains(point))
                    .map(|patch| patch.evaluated.pixel(point, [0.; 3]));
                if let Some(value) = value {
                    assert_eq!(
                        base[3], actual[3],
                        "source floor/validity flags must survive replacement"
                    );
                    if matches!(format, Format::Exr(_)) {
                        let global = crate::photographic::Grade::new(photo)
                            .unwrap()
                            .apply(global_matrix * DVec3::from_array(value.map(f64::from)))
                            .unwrap();
                        let expected = locals
                            .apply(
                                InvertedPixel {
                                    rgb: global,
                                    floored: [false; 3],
                                },
                                SourcePoint(point.map(|v| v as f64 + 0.5)),
                            )
                            .unwrap()
                            .rgb;
                        for c in 0..3 {
                            assert!(
                                (actual[c] - expected[c]).abs()
                                    <= 2e-12 + 2e-11 * expected[c].abs(),
                                "{point:?} channel {c}: {} vs {}",
                                actual[c],
                                expected[c]
                            );
                        }
                    }
                } else {
                    assert_eq!(
                        base.map(f64::to_bits),
                        actual.map(f64::to_bits),
                        "untouched original at {point:?}"
                    );
                }
            }
            singles.extend(actual);
        }
        let geometry = SourceGeometry::original_pair([first, second], TILE).unwrap();
        let mut pair = OriginalTiles::new_geometry(
            &renderer,
            (&device, &queue, &gate),
            (DMat3::IDENTITY, geometry.size()),
            None,
            Arc::new(grade()),
            (format, None),
            Some(geometry),
        )
        .unwrap();
        pair.set_repairs(&device, repairs.clone()).unwrap();
        let mut pixels = tests::tile(first, TILE);
        pixels.extend(tests::tile(second, TILE));
        let paired = pair
            .read_geometry(&renderer, &device, &queue, &gate, Some(geometry), &pixels)
            .unwrap();
        assert_eq!(
            singles
                .iter()
                .map(|p| p.map(f64::to_bits))
                .collect::<Vec<_>>(),
            paired
                .iter()
                .map(|p| p.map(f64::to_bits))
                .collect::<Vec<_>>()
        );
    }
}
