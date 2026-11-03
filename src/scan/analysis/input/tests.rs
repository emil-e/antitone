use super::*;

#[test]
fn capture_owns_original_codes_without_a_native_lease_and_counts_its_payload() {
    let file = crate::raw::fixture::bayer_dng_with_rgb([6000, 12000, 9000]);
    let scan = Scan::open(file.path(), None).unwrap();
    let native = Arc::downgrade(scan.source());
    let input = AnalysisInput::capture(&scan, 16, || false)
        .unwrap()
        .unwrap();
    assert_eq!(input.patches.len(), 16);
    assert_eq!(
        input.retained_bytes(),
        (size_of::<AnalysisInput>() + 16 * size_of::<Patch>()) as u64
    );
    let image = scan.source().camera_image().unwrap();
    for patch in &input.patches {
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(
                    patch.pixels[(y * 8 + x) as usize],
                    image
                        .pixel_samples(patch.footprint.region.x + x, patch.footprint.region.y + y)
                        .unwrap()
                );
            }
        }
    }
    drop(scan);
    assert!(
        native.upgrade().is_none(),
        "retained patches must not pin a full original"
    );
    assert_eq!(
        input
            .plan(input.area, 16, |_| true, &mut || false)
            .unwrap()
            .unwrap()
            .footprints
            .len(),
        16
    );
}

#[test]
fn native_floor_decisions_are_widened_before_division_and_masks_precede_rejection() {
    let file = crate::raw::fixture::bayer_dng_with_rgb([2; 3]);
    let mut scan = Scan::open(file.path(), None).unwrap();
    let pixel = scan
        .source
        .camera_image()
        .unwrap()
        .pixel(scan.area.x + 12, scan.area.y + 12)
        .unwrap()
        .min_element();
    Arc::make_mut(&mut scan.source).input_floor = f64::from_bits(pixel.to_bits() - 1);
    let mut input = AnalysisInput::capture(&scan, 16, || false)
        .unwrap()
        .unwrap();
    let direct = scan
        .analysis_plan(scan.area, 16, |_| true, &mut || false)
        .unwrap()
        .unwrap();
    let retained = input
        .plan(scan.area, 16, |_| true, &mut || false)
        .unwrap()
        .unwrap();
    assert!(
        !direct.footprints.is_empty(),
        "native floor control must retain values just above the f64 bound"
    );
    assert_eq!(direct.samples.attempted, retained.samples.attempted);
    assert_eq!(
        direct.samples.rejected_floor,
        retained.samples.rejected_floor
    );
    assert_eq!(direct.footprints.len(), retained.footprints.len());
    input.patches[0].nonfinite = true;
    let excluded = input.patches[0].footprint.region;
    assert!(
        input
            .plan(scan.area, 16, |f| f != excluded, &mut || false)
            .unwrap()
            .is_some()
    );
    assert!(input.plan(scan.area, 16, |_| true, &mut || false).is_err());
    assert!(
        AnalysisInput::capture(&scan, 16, || true)
            .unwrap()
            .is_none()
    );
    assert!(
        input
            .plan(scan.area, 16, |_| true, &mut || true)
            .unwrap()
            .is_none()
    );
}
