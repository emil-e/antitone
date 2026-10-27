use super::*;
use crate::{
    dust_repair::{DonorChoice, RepairId, RepairOrigin, SupportMask},
    processing::{Calibration, InputSettings, Pipeline, RollSettings},
    source::SourceCache,
};
use glam::DVec3;

#[test]
fn repair_sampling_sees_factory_normalized_ap1_but_negative_and_native_samples_do_not_change() {
    let file = crate::exposure_fixture::neutral_dng_with_defect(1.3);
    let scan = Scan::open(file.path(), Some([1.; 4])).unwrap();
    let repair = Repair {
        id: RepairId(1),
        enabled: true,
        origin: RepairOrigin::Manual,
        support: SupportMask::new([10, 10], [13, 13], vec![true; 13 * 13]).unwrap(),
        donor: DonorChoice::Offset([31, 31]),
    };
    let negative: InputSettings = Calibration {
        dmin: DVec3::splat(0.2),
        dmax: DVec3::splat(0.01),
        input_floor: crate::processing::CAMERA_INPUT_FLOOR,
    }
    .into();
    let before = evaluate(&ImageSource::new(&scan, negative).unwrap(), &repair).unwrap();
    let samples = scan.pixel(48, 48).unwrap();
    assert!(
        scan.pixel(16, 16).unwrap().max_element() < samples.min_element() * 0.1,
        "fixture must contain an actual sensor defect, not merely a mask"
    );
    let prepared = SourceCache::new(0)
        .prepare_source(InputSettings::Digital, scan.source())
        .unwrap();
    assert!(
        matches!(
            prepared.source.reference_exposure.get(),
            Some(crate::exposure_reference::ReferenceExposure::Matched(_))
        ),
        "{:?}",
        prepared.source.reference_exposure.get()
    );
    let normalized = Scan::from_source(prepared.source);
    let evaluated = evaluate(
        &ImageSource::new(&normalized, InputSettings::Digital).unwrap(),
        &repair,
    )
    .unwrap();
    let expected = 6000. / 65535. * 1.3_f64.exp2();
    let mut checked = 0;
    for (_, pixel) in evaluated.pixels() {
        for value in pixel {
            assert!(
                (f64::from(value) / expected - 1.).abs() < 0.03,
                "{pixel:?} vs {expected}"
            );
        }
        checked += 1;
    }
    assert_eq!(checked, 169);
    assert_eq!(
        scan.pixel(48, 48).unwrap(),
        samples,
        "normalization must not rewrite native camera samples"
    );
    let after = evaluate(&ImageSource::new(&normalized, negative).unwrap(), &repair).unwrap();
    assert_eq!(
        before.pixels().collect::<Vec<_>>(),
        after.pixels().collect::<Vec<_>>()
    );
    let grade = Pipeline::new(
        InputSettings::Digital,
        RollSettings::default(),
        normalized.positive_matrix(&InputSettings::Digital).unwrap(),
    )
    .unwrap();
    for (_, pixel) in evaluated.pixels() {
        let rgb = DVec3::from_array(pixel.map(f64::from));
        assert_eq!(
            grade
                .apply_roll(crate::processing::InvertedPixel {
                    rgb,
                    floored: [false; 3]
                })
                .unwrap()
                .rgb,
            rgb
        );
    }
}
