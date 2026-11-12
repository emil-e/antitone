#![cfg(feature = "raw")]
use antitone::raw::RawFile;
use glam::DVec3;

#[path = "support/raw_fixture.rs"]
mod fixture;
#[path = "support/fixture_path.rs"]
mod fixture_path;

#[test]
fn libraw_clipped_colour_keeps_floating_headroom_and_repeats_in_both_output_balances() {
    let file = fixture::bayer_dng_with_as_shot([58982, 65535, 65535], [2, 1, 2]);
    // Diagnostic uncorrected opening selects LibRaw even in SDK-enabled builds.
    let mut raw = RawFile::open_uncorrected(file.path()).unwrap();
    assert_eq!(raw.metadata.as_shot_wb(), [2., 1., 2., 1.]);
    let expected_mean = (2. * 58982. / 65535. + 1. + 2.) / 3.;
    let mut previous = None;
    for wb in [[2., 1., 2., 1.], [1.; 4], [2., 1., 2., 1.]] {
        let developed = raw.develop_camera(wb).unwrap();
        let value = developed.pixel(32, 32).unwrap();
        let scale = wb[..3].iter().copied().fold(0., f64::max) / wb[1];
        assert!(
            developed.is_continuous(),
            "changed pixels must not be rounded/clipped back to RGB16"
        );
        let expected = DVec3::new(
            expected_mean * wb[0] / 2.,
            expected_mean * wb[1],
            expected_mean * wb[2] / 2.,
        );
        // At this measured-red site, scaled DHT green is 32766 versus white
        // 32767.5. Blend's green shoulder amplifies that error; the SDK float
        // fixture independently tests the exact neutral without integer scaling.
        let error = (value * scale - expected).abs();
        assert!(
            error.x < 4. / 65535. && error.y < 8. / 65535. && error.z < 4. / 65535.,
            "{value:?} vs {expected:?}"
        );
        if wb == [1.; 4] {
            assert!(
                value.y > 1.,
                "unity development must retain reconstructed sensor overrange"
            );
        }
        if wb == [2., 1., 2., 1.] {
            if let Some(previous) = previous {
                assert_eq!(
                    value, previous,
                    "repeat development cannot compound repair or retain stale floats"
                );
            }
            previous = Some(value);
        }
    }
}

#[test]
fn ordinary_unity_source_keeps_its_declared_camera_codes() {
    let file = fixture::bayer_dng();
    let mut raw = RawFile::open_uncorrected(file.path()).unwrap();
    let image = raw.develop_camera([1.; 4]).unwrap();
    assert!(!image.is_continuous());
    assert_eq!(
        image.pixel_codes(32, 32).unwrap()[..3],
        [8000, 16000, 24000]
    );
}

#[test]
fn libraw_unclipped_nearwhite_colour_remains_integer_and_unchanged() {
    for signal in [[58982, 63569, 52428], [58982, 65207, 52428]] {
        let file = fixture::bayer_dng_with_as_shot(signal, [2, 1, 2]);
        let mut raw = RawFile::open_uncorrected(file.path()).unwrap();
        for wb in [[2., 1., 2., 1.], [1.; 4]] {
            let developed = raw.develop_camera(wb).unwrap();
            assert!(
                !developed.is_continuous(),
                "a wholly unclipped frame must not allocate a repaired float source"
            );
            let scale = if wb == [1.; 4] { 1. } else { 2. };
            let expected = DVec3::from_array(signal.map(|v| f64::from(v) / 65535.))
                * DVec3::new(wb[0], wb[1], wb[2]);
            assert!(
                (developed.pixel(32, 32).unwrap() * scale - expected)
                    .abs()
                    .max_element()
                    < 2. / 65535.
            );
        }
    }
}

#[test]
fn distant_clipping_does_not_change_colour_below_the_common_balanced_white() {
    // Every capture-balanced component is below common white, despite .995 sensor green.
    let signal = [32702, 65207, 32112];
    let file = fixture::bayer_dng_with_as_shot_and_clipped_corner(signal, [2, 1, 2]);
    let mut raw = RawFile::open_uncorrected(file.path()).unwrap();
    for wb in [[2., 1., 2., 1.], [1.; 4]] {
        let developed = raw.develop_camera(wb).unwrap();
        let scale = if wb == [1.; 4] { 1. } else { 2. };
        let expected = DVec3::from_array(signal.map(|v| f64::from(v) / 65535.))
            * DVec3::new(wb[0], wb[1], wb[2]);
        assert!(
            (developed.pixel(48, 48).unwrap() * scale - expected)
                .abs()
                .max_element()
                < 4. / 65535.
        );
    }
}

#[test]
fn blend_reaches_dht_clipping_deficits_in_both_wb_bases() {
    let file = fixture::bayer_dng_with_as_shot_samples([2, 1, 2], |x, y| {
        [if x == 32 && y == 32 { 56600 } else { 60000 }, 65535, 60000]
    });
    let mut raw = RawFile::open_uncorrected(file.path()).unwrap();
    for wb in [[2., 1., 2., 1.], [1.; 4]] {
        let image = raw.develop_camera(wb).unwrap();
        let scale = if wb == [1.; 4] { 1. } else { 2. };
        let sensor = image.pixel(32, 32).unwrap() * scale / DVec3::new(wb[0], wb[1], wb[2]);
        assert!(image.is_continuous());
        assert!(
            sensor.y > 1.1,
            "Blend must actually reach the DHT hole: {sensor:?}"
        );
    }
}

#[test]
fn bright_unclipped_primaries_do_not_acquire_pale_cores_in_a_clipped_frame() {
    for dominant in 0..3 {
        let mut signal = [0; 3];
        signal[dominant] = 58982;
        let file = fixture::bayer_dng_with_as_shot_and_clipped_corner(signal, [2, 1, 2]);
        let mut raw = RawFile::open_uncorrected(file.path()).unwrap();
        for wb in [[2., 1., 2., 1.], [1.; 4]] {
            let image = raw.develop_camera(wb).unwrap();
            let colour = image.pixel(48, 48).unwrap();
            assert!(
                colour[dominant] > 0.25,
                "a black result is not colour preservation"
            );
            for c in 0..3 {
                if c != dominant {
                    assert!(
                        colour[c].abs() < 2. / 65535.,
                        "clipping elsewhere cannot turn a saturated primary pale: {colour:?}"
                    );
                }
            }
        }
    }
}
