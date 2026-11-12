use super::*;
use antitone::dng::Source as SdkSource;

fn develop(signal: [u16; 3], warp: bool, wb: [f64; 3]) -> SdkSource {
    let mut fixture = Fixture {
        signal,
        neutral: [(1, 2), (1, 1), (2, 3)],
        ..Default::default()
    };
    if warp {
        fixture.lists[2].push(Opcode::warp(&[[0.98, 0.015, 0., 0., 0., 0.]], [0.5, 0.5]));
    }
    let file = fixture.write();
    let source = SdkSource::develop(
        file.path(),
        Options {
            max_pixels: 4096,
            white_balance: Some(wb),
        },
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(
        source.opcode_counts().0,
        [0, 0, u32::from(warp)],
        "warp controls must execute, not silently skip"
    );
    source
}

fn assert_same_worker_result(serial: &SdkSource, parallel: &SdkSource) {
    assert_eq!(parallel.header(), serial.header());
    assert_eq!(parallel.opcode_counts(), serial.opcode_counts());
    assert!(
        parallel
            .camera_rgb()
            .iter()
            .flat_map(|pixel| pixel.map(f32::to_bits))
            .eq(serial
                .camera_rgb()
                .iter()
                .flat_map(|pixel| pixel.map(f32::to_bits))),
        "worker budgets must preserve every completed camera bit"
    );
}

#[test]
fn clipped_camera_colour_is_repaired_before_warp_without_losing_headroom_in_either_wb_basis() {
    // Encoded signal (.89990234375,1,1) with WB (2,1,1.5).
    // All channel shoulders fully select the neutral candidate, whose balanced
    // mean is 1.433268229... below the 1.5 ceiling, rather than SDK-clipped green=1.
    let neutral = (2. * 3686. / 4096. + 1. + 1.5) / 3.;
    for warp in [false, true] {
        for wb in [[2., 1., 1.5], [1.; 3]] {
            let source = develop([3686, 4096, 4096], warp, wb);
            let expected = [neutral * wb[0] / 2., neutral * wb[1], neutral * wb[2] / 1.5];
            let [w, h] = source.dimensions();
            assert_eq!(source.camera_rgb().len(), w as usize * h as usize);
            for pixel in source.camera_rgb() {
                for c in 0..3 {
                    assert!(
                        (f64::from(pixel[c]) - expected[c]).abs() < 2e-6,
                        "warp={warp}, WB={wb:?}: {pixel:?} vs {expected:?}"
                    );
                }
            }
            assert!(
                source.camera_rgb()[0][1] > 1.,
                "reconstructed overrange cannot be clipped by the warp or final output"
            );
        }
    }
}

#[test]
fn unclipped_nearwhite_colour_keeps_its_sensor_values_and_output_balance() {
    // Include bright .995 colour without an original-frame clipping anchor.
    for signal in [[1638, 2662, 1024], [3686, 3973, 3277], [3686, 4076, 3277]] {
        for warp in [false, true] {
            for wb in [[2., 1., 1.5], [1.; 3]] {
                let source = develop(signal, warp, wb);
                for pixel in source.camera_rgb() {
                    for c in 0..3 {
                        let expected = f64::from(signal[c]) / 4096. * wb[c];
                        assert!(
                            (f64::from(pixel[c]) - expected).abs() < 2e-6,
                            "unclipped colour must not be desaturated: {pixel:?}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn depressed_clipped_codes_lose_the_cast_while_below_common_white_colour_is_unchanged() {
    for signal in [[3686, 4096, 3277], [2044, 4076, 2728]] {
        for warp in [false, true] {
            let mut fixture = Fixture {
                signal,
                clipped_corner: true,
                neutral: [(1, 2), (1, 1), (2, 3)],
                ..Default::default()
            };
            let depressed = signal[1] == 4096;
            let gain = if depressed { f64::from(0.995f32) } else { 1. };
            if depressed {
                // Green clips at the sensor white; a real list-2 gain then lowers
                // the foreground codes, leaving the corner as an exact white anchor.
                let mut opcode = Opcode::gain(0.995, 1);
                opcode.data[..4].copy_from_slice(&16u32.to_be_bytes());
                fixture.lists[1].push(opcode);
            }
            if warp {
                fixture.lists[2].push(Opcode::warp(&[[0.98, 0.015, 0., 0., 0., 0.]], [0.5, 0.5]));
            }
            let file = fixture.write();
            for wb in [[2., 1., 1.5], [1.; 3]] {
                let source = SdkSource::develop(
                    file.path(),
                    Options {
                        max_pixels: 4096,
                        white_balance: Some(wb),
                    },
                    &Cancellation::default(),
                )
                .unwrap();
                assert_eq!(
                    source.opcode_counts().0,
                    [0, u32::from(depressed), u32::from(warp)]
                );
                let [w, h] = source.dimensions();
                let pixel = source.camera_rgb()[((h / 2) * w + w / 2) as usize];
                let actual = DVec3::from_array(pixel.map(f64::from));
                let expected = DVec3::from_array(signal.map(|v| f64::from(v) / 4096.))
                    * gain
                    * DVec3::from_array(wb);
                if depressed {
                    let balanced = actual * DVec3::new(2. / wb[0], 1. / wb[1], 1.5 / wb[2]);
                    let original = DVec3::from_array(signal.map(|v| f64::from(v) / 4096.))
                        * gain
                        * DVec3::new(2., 1., 1.5);
                    assert!(
                        balanced.max_element() - balanced.min_element()
                            < (original.max_element() - original.min_element()) * 0.5,
                        "the real gain-depressed clipping case must materially lose its cast"
                    );
                    assert!(balanced.y > 1.2, "cleanup must retain above-white light");
                } else {
                    // The second colour is below common balanced white, despite
                    // its near-white sensor green and the frame's clipped corner.
                    assert!((actual - expected).abs().max_element() < 2e-6);
                }
            }
        }
    }
}

#[test]
fn pointwise_blend_handles_below_white_rows_across_sdk_tiles_and_workers() {
    for top in [64u32, 127] {
        let mut fixture = Fixture {
            side: 1024,
            signal: [3686, 4096, 3277],
            neutral: [(1, 2), (1, 1), (2, 3)],
            ..Default::default()
        };
        let mut opcode = Opcode::gain(0.97, 1);
        // Stage-2 rows above the gain retain the real sensor-clipped green.
        // Row 64 also starts a new dirty tile; row 127 splits an eight-worker run.
        opcode.data[..4].copy_from_slice(&top.to_be_bytes());
        opcode.data[8..12].copy_from_slice(&1016u32.to_be_bytes());
        opcode.data[12..16].copy_from_slice(&1016u32.to_be_bytes());
        fixture.lists[1].push(opcode);
        let file = fixture.write();
        for wb in [[2., 1., 1.5], [1.; 3]] {
            let options = Options {
                max_pixels: 1024 * 1024,
                white_balance: Some(wb),
            };
            let serial =
                SdkSource::develop_with_threads(file.path(), options, &Cancellation::default(), 1)
                    .unwrap();
            let source =
                SdkSource::develop_with_threads(file.path(), options, &Cancellation::default(), 8)
                    .unwrap();
            assert_same_worker_result(&serial, &source);
            assert_eq!(source.opcode_counts().0, [0, 1, 0]);
            let [w, h] = source.dimensions();
            assert_eq!([w, h], [1016; 2]);
            let pixel = |y: u32| {
                DVec3::from_array(source.camera_rgb()[(y * w + w / 2) as usize].map(f64::from))
            };
            let sensor = pixel(top) / DVec3::from_array(wb);
            assert!(
                sensor.y > 1.2,
                "below-white input at a tile/worker boundary must actually be blended"
            );
            let interior = pixel(top + 2);
            assert!(
                interior.y / wb[1] > 1.2,
                "a uniform below-white region must be treated beyond the boundary"
            );
            for y in [top + 16, top + 65] {
                assert!(
                    (pixel(y) - interior).abs().max_element() < 2e-6,
                    "uniform input must not gain a dirty-tile or worker seam: y={y}"
                );
            }
        }
    }
}

#[test]
fn active_blend_and_warp_headroom_match_across_worker_budgets() {
    let mut fixture = Fixture {
        side: 1031,
        ramp: true,
        signal: [3686, 4096, 4096],
        neutral: [(1, 2), (1, 1), (2, 3)],
        ..Default::default()
    };
    fixture.lists[2] = vec![
        Opcode::warp(&[[0.98, 0.015, 0., 0., 0., 0.]], [0.5, 0.5]),
        Opcode::warp(&[[0.99, 0.008, 0., 0., 0., 0.]], [0.45, 0.55]),
    ];
    let file = fixture.write();
    for wb in [[2., 1., 1.5], [1.; 3]] {
        let options = Options {
            max_pixels: 2_000_000,
            white_balance: Some(wb),
        };
        let serial =
            SdkSource::develop_with_threads(file.path(), options, &Cancellation::default(), 1)
                .unwrap();
        let parallel =
            SdkSource::develop_with_threads(file.path(), options, &Cancellation::default(), 8)
                .unwrap();
        assert_same_worker_result(&serial, &parallel);
        assert_eq!(parallel.opcode_counts().0, [0, 0, 2]);
        assert!(
            parallel.timings().sdk_workers > 1,
            "the large warped control must actually use SDK workers"
        );
        let [w, h] = parallel.dimensions();
        let centre = parallel.camera_rgb()[((h / 2) * w + w / 2) as usize];
        let green = f64::from(centre[1]) / wb[1];
        assert!(
            green > 1.1,
            "a no-op or clipped warp is not active parallel headroom coverage"
        );
    }
}

#[test]
fn missing_capture_balance_leaves_colour_untreated_independently_of_requested_wb() {
    let fixture = Fixture {
        signal: [3686, 4096, 4096],
        omit_neutral: true,
        ..Default::default()
    };
    let file = fixture.write();
    for wb in [[1.; 3], [2., 1., 1.5]] {
        let options = Options {
            max_pixels: 4096,
            white_balance: Some(wb),
        };
        let header = Header::read(file.path(), options, &Cancellation::default()).unwrap();
        assert_eq!(
            header.as_shot_wb, [0.; 3],
            "fixture must actually omit source colour evidence"
        );
        let source = SdkSource::develop(file.path(), options, &Cancellation::default()).unwrap();
        for pixel in source.camera_rgb() {
            for c in 0..3 {
                let expected = [3686. / 4096., 1., 1.][c] * wb[c];
                assert!(
                    (f64::from(pixel[c]) - expected).abs() < 2e-6,
                    "unknown repair basis cannot be invented from output WB"
                );
            }
        }
    }
}

#[test]
fn admitted_extreme_output_balance_still_loads_when_repair_exceeds_the_source_domain() {
    let fixture = Fixture {
        signal: [3686, 4096, 4096],
        neutral: [(1, 2), (1, 1), (2, 3)],
        ..Default::default()
    };
    let file = fixture.write();
    let options = Options {
        max_pixels: 4096,
        white_balance: Some([1., 256., 1.]),
    };
    Header::read(file.path(), options, &Cancellation::default()).unwrap();
    let source = SdkSource::develop(file.path(), options, &Cancellation::default()).unwrap();
    for pixel in source.camera_rgb() {
        assert_eq!(
            *pixel,
            [3686. / 4096., 256., 1.],
            "unsupported repair headroom must preserve the valid baseline rather than reject loading"
        );
    }
}

#[test]
fn spatial_white_evidence_is_corrected_before_interpolation_and_survives_two_warps() {
    let mut fixture = Fixture {
        side: 512,
        ramp: true,
        signal: [3686, 4096, 4096],
        neutral: [(1, 2), (1, 1), (2, 3)],
        ..Default::default()
    };
    fixture.lists[2] = vec![
        Opcode::warp(&[[0.98, 0.015, 0., 0., 0., 0.]], [0.5, 0.5]),
        Opcode::warp(&[[0.99, 0.008, 0., 0., 0., 0.]], [0.45, 0.55]),
    ];
    let file = fixture.write();
    let source = SdkSource::develop(
        file.path(),
        Options {
            max_pixels: 512 * 512,
            white_balance: None,
        },
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(source.opcode_counts().0, [0, 0, 2]);
    let [w, h] = source.dimensions();
    let mut minimum = f32::INFINITY;
    let mut maximum = f32::NEG_INFINITY;
    let mut maximum_chroma = 0_f32;
    for y in 8..h - 8 {
        for x in 8..w - 8 {
            let p = source.camera_rgb()[(y * w + x) as usize];
            minimum = minimum.min(p[0]);
            maximum = maximum.max(p[0]);
            maximum_chroma = maximum_chroma.max(
                p.into_iter().fold(f32::NEG_INFINITY, f32::max)
                    - p.into_iter().fold(f32::INFINITY, f32::min),
            );
        }
    }
    assert!(
        maximum - minimum > 0.02,
        "surviving red variation must distinguish this fixture from a constant plateau"
    );
    assert!(
        maximum_chroma < 0.01,
        "early repair must keep a neutral highlight through both spatial interpolations; late exact-white detection leaves large coloured holes: {maximum_chroma}"
    );
}

#[test]
fn repaired_development_is_immutable_after_its_native_transaction_and_file_are_gone() {
    let source = develop([3686, 4096, 4096], true, [1.; 3]);
    let expected = source.camera_rgb().to_vec();
    let source = Arc::new(source);
    let lease = source.clone();
    drop(source);
    std::thread::spawn(move || {
        assert_eq!(
            lease.camera_rgb(),
            expected,
            "completed source owns repaired floats after SDK workers/negative are gone"
        );
    })
    .join()
    .unwrap();
}
