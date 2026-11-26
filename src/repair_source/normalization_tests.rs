use super::*;
use crate::scan::DevelopedSource;
use crate::{
    dust_repair::{RepairId, RepairOrigin, SupportMask},
    processing::{Calibration, InputSettings, Pipeline, RollSettings},
};
use glam::DVec3;

#[test]
fn repair_sampling_sees_factory_normalized_ap1_but_negative_and_native_samples_do_not_change() {
    let file = crate::exposure_fixture::neutral_dng_with_defect(1.3);
    let scan = DevelopedSource::open(file.path(), Some([1.; 4])).unwrap();
    let repair = Repair {
        id: RepairId(1),
        enabled: true,
        origin: RepairOrigin::Manual,
        support: SupportMask::new([10, 10], [13, 13], vec![true; 13 * 13]).unwrap(),
        offset: [31, 31],
    };
    let negative: InputSettings = Calibration {
        dmin: DVec3::splat(0.2),
        dmax: DVec3::splat(0.01),
    }
    .into();
    let before = evaluate(
        &ImageSource::new(&scan, negative, None, None).unwrap(),
        &repair,
    )
    .unwrap();
    let samples = scan.pixel(48, 48).unwrap();
    assert!(
        scan.pixel(16, 16).unwrap().max_element() < samples.min_element() * 0.1,
        "fixture must contain an actual sensor defect, not merely a mask"
    );
    let prepared = crate::photo_preparation::native(
        None,
        &std::sync::OnceLock::new(),
        InputSettings::Digital,
        &scan,
    )
    .unwrap();
    assert!(
        matches!(
            prepared.reference.as_ref(),
            Some(crate::exposure_reference::ReferenceExposure::Matched(_))
        ),
        "{:?}",
        prepared.reference.as_ref()
    );
    let reference_ev = prepared.reference.as_ref().map(|reference| reference.ev());
    let evaluated = evaluate(
        &ImageSource::new(&prepared.source, InputSettings::Digital, reference_ev, None).unwrap(),
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
    let after = evaluate(
        &ImageSource::new(&prepared.source, negative, reference_ev, None).unwrap(),
        &repair,
    )
    .unwrap();
    assert_eq!(
        before.pixels().collect::<Vec<_>>(),
        after.pixels().collect::<Vec<_>>()
    );
    let grade = Pipeline::new(
        InputSettings::Digital,
        RollSettings::default(),
        InputSettings::Digital
            .positive_matrix(prepared.source.camera_to_ap1(), reference_ev)
            .unwrap(),
    )
    .unwrap();
    for (_, pixel) in evaluated.pixels() {
        let rgb = DVec3::from_array(pixel.map(f64::from));
        assert_eq!(
            grade
                .apply_roll(crate::processing::InvertedPixel { rgb })
                .unwrap()
                .rgb,
            rgb
        );
    }
}
