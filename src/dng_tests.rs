use super::*;
use glam::DVec3;
use std::sync::atomic::AtomicUsize;

use crate::dng_fixture as fixture;
use fixture::{Fixture, Opcode};

#[path = "dng_execution_tests.rs"]
mod execution;

fn options() -> Options {
    Options {
        max_pixels: 4096,
        fixed_wb: Some([1.; 3]),
    }
}

fn develop(fixture: &Fixture) -> Source {
    Source::develop(fixture.write().path(), options(), &Cancellation::default()).unwrap()
}

fn pixel(source: &Source, x: usize, y: usize) -> [f32; 3] {
    source.camera_rgb()[y * source.dimensions()[0] as usize + x]
}

fn warp(plane: bool) -> Opcode {
    let mut coefficients = vec![[1., 0.02, 0., 0., 0.002, -0.003]];
    if plane {
        coefficients.extend([[1., 0., 0., 0., 0., 0.], [1., -0.02, 0., 0., 0., 0.]]);
    }
    Opcode::warp(&coefficients, [0.35, 0.65])
}

#[test]
fn owned_basis_retains_original_codes_crop_orientation_and_fixed_wb() {
    let fixture = Fixture {
        orientation: 6,
        neutral: [(1, 2), (1, 1), (2, 3)],
        ..Default::default()
    };
    let file = fixture.write();
    let source = Source::develop(
        file.path(),
        Options {
            fixed_wb: None,
            ..options()
        },
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(source.dimensions(), [56, 56]);
    assert_eq!(source.original_dimensions(), [64, 64]);
    assert_eq!(source.active_origin(), [6, 4]);
    assert_eq!(source.default_crop(), [3, 5, 48, 46]);
    assert_eq!(source.tiff_orientation(), 6);
    assert_eq!(source.capture_limits(), [64., 4160.]);
    assert_eq!(source.fixed_wb(), [2., 1., 1.5]);
    assert_eq!(pixel(&source, 20, 20), [0.5, 0.375, 0.75]);
    assert_eq!(source.original_cfa()[28 * 64 + 30], 1088);
    assert!(source.capture_state(20, 20).unwrap().eligible());
    assert!(source.capture_state(0, 0).unwrap().repeated_border());
    assert!(source.capture_state(56, 0).is_none());
    assert!(source.retained_bytes() >= (56 * 56 * 13 + 64 * 64 * 2) as u64);
    // A camera-neutral sample becomes balanced ones, then XYZ D50 without a second WB.
    let white = source.camera_to_xyz_d50() * DVec3::ONE;
    let expected = DVec3::new(0.3457 / 0.3585, 1., (1. - 0.3457 - 0.3585) / 0.3585);
    assert!((white - expected).abs().max_element() < 2e-6, "{white:?}");
    drop(file);
    let source = Arc::new(source);
    let other = source.clone();
    assert_eq!(
        std::thread::spawn(move || pixel(&other, 20, 20))
            .join()
            .unwrap(),
        [0.5, 0.375, 0.75]
    );
    assert_eq!(source.original_cfa()[28 * 64 + 30], 1088);
}

#[test]
fn fixed_basis_does_not_follow_later_as_shot_metadata() {
    let a = Fixture::default();
    let b = Fixture {
        neutral: [(1, 2), (1, 1), (2, 3)],
        ..Default::default()
    };
    let a = develop(&a);
    let b = develop(&b);
    assert_eq!(a.camera_rgb(), b.camera_rgb());
    assert_eq!(a.camera_to_xyz_d50(), b.camera_to_xyz_d50());
    assert_eq!(a.interpretation_id(), b.interpretation_id());
}

#[test]
fn gain_order_keeps_each_sdk_clip_and_wb_is_after_opcodes_without_another_clip() {
    for (gains, expected) in [([4., 0.5], [0.5, 0.5, 0.5]), ([0.5, 4.], [0.5, 0.75, 1.])] {
        let mut fixture = Fixture::default();
        fixture.lists[1] = gains.into_iter().map(|v| Opcode::gain(v, 1)).collect();
        let source = develop(&fixture);
        assert_eq!(pixel(&source, 20, 20), expected);
        assert!(source.capture_state(20, 20).unwrap().processing_limit());
        assert!(!source.capture_state(20, 20).unwrap().original_white());
        assert_eq!(source.opcode_counts().0, [0, 2, 0]);
    }
    let mut fixture = Fixture::default();
    fixture.lists[1].push(Opcode::gain(4., 1));
    let file = fixture.write();
    let source = Source::develop(
        file.path(),
        Options {
            fixed_wb: Some([2., 1., 1.5]),
            ..options()
        },
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(pixel(&source, 20, 20), [2., 1., 1.5]);
    assert!(!source.capture_state(20, 20).unwrap().original_white());
    assert!(source.capture_state(20, 20).unwrap().processing_limit());
}

#[test]
fn list1_gain_precedes_black_subtraction_and_does_not_replace_original_codes() {
    let mut fixture = Fixture::default();
    fixture.lists[0].push(Opcode::gain(2., 0));
    let source = develop(&fixture);
    let expected =
        [0.25_f32, 0.375, 0.5].map(|v| ((v * 4096. + 64.) * 2. - 64.).min(4096.) / 4096.);
    assert_eq!(pixel(&source, 20, 20), expected);
    assert_eq!(source.original_cfa()[28 * 64 + 30], 1088);
}

#[test]
fn identity_is_exact_and_nonidentity_geometry_matches_the_declared_affine_ramp() {
    let base = Fixture {
        ramp: true,
        ..Default::default()
    };
    let source = develop(&base);
    let mut identity = Fixture {
        ramp: true,
        ..Default::default()
    };
    identity.lists[2].push(Opcode::warp(&[[1., 0., 0., 0., 0., 0.]], [0.5, 0.5]));
    let identity = develop(&identity);
    assert_eq!(source.camera_rgb(), identity.camera_rgb());
    for per_plane in [false, true] {
        let mut fixture = Fixture {
            ramp: true,
            ..Default::default()
        };
        fixture.lists[2].push(warp(per_plane));
        let source = develop(&fixture);
        // Chapter-7 coordinate equations, not an SDK resampler reimplementation.
        // A=-.75's first-moment error <=.048113 pixel plus <1/32 phase-floor error.
        let tolerance = (0.048113 + 1. / 32.) * 6. / 4096. + 1e-6;
        let (cx, cy) = (0.35_f64 * 56., 0.65_f64 * 56.);
        let radius = cx.max(56. - cx).hypot(cy.max(56. - cy));
        for y in 8..48 {
            for x in 8..48 {
                let dx = (x as f64 - cx) / radius;
                let dy = (y as f64 - cy) / radius;
                let r2 = dx * dx + dy * dy;
                for (p, base) in [0.25, 0.375, 0.5].into_iter().enumerate() {
                    let (radial, t0, t1) = if per_plane && p > 0 {
                        (if p == 1 { 0. } else { -0.02 }, 0., 0.)
                    } else {
                        (0.02, 0.002, -0.003)
                    };
                    let sx = cx
                        + radius
                            * ((1. + radial * r2) * dx
                                + 2. * t0 * dx * dy
                                + t1 * (r2 + 2. * dx * dx));
                    let sy = cy
                        + radius
                            * ((1. + radial * r2) * dy
                                + 2. * t1 * dx * dy
                                + t0 * (r2 + 2. * dy * dy));
                    let expected = base + (4. * (sx + 6.) + 2. * (sy + 4.)) / 4096.;
                    assert!((pixel(&source, x, y)[p] as f64 - expected).abs() <= tolerance);
                }
            }
        }
    }
}

#[test]
fn changed_sensor_sites_cannot_hide_behind_interpolation_or_sequential_warps() {
    for count in [0, 1, 2] {
        for (x, y) in [(30, 28), (31, 29), (30, 29), (6, 4), (61, 59)] {
            for code in [0, 4160] {
                let mut fixture = Fixture::default();
                for _ in 0..count {
                    fixture.lists[2].push(warp(true));
                }
                let baseline = develop(&fixture);
                fixture.marker = Some((x, y, code));
                let changed = develop(&fixture);
                assert_eq!(changed.original_cfa()[(y * 64 + x) as usize], code);
                let mut affected = 0;
                for (i, (a, b)) in baseline
                    .camera_rgb()
                    .iter()
                    .zip(changed.camera_rgb())
                    .enumerate()
                {
                    if a != b {
                        affected += 1;
                        let flags = changed.capture_state(i as u32 % 56, i as u32 / 56).unwrap();
                        assert!(
                            if code == 0 {
                                flags.original_black()
                            } else {
                                flags.original_white()
                            },
                            "unflagged dependency: warps={count} site=({x},{y}) output={i}"
                        );
                    }
                }
                assert!(affected > 0);
                assert!(changed.capture_state(12, 12).unwrap().eligible());
            }
        }
    }
}

#[test]
fn a_later_warp_cannot_hide_new_border_clamping_behind_an_eligible_prior_edge() {
    let mut fixture = Fixture::default();
    fixture.lists[2].push(Opcode::warp(&[[0.5, 0., 0., 0., 0., 0.]], [0.5, 0.5]));
    let contracted = develop(&fixture);
    assert!(contracted.capture_state(0, 28).unwrap().eligible());
    fixture.lists[2].push(Opcode::warp(&[[1.05, 0., 0., 0., 0., 0.]], [0.5, 0.5]));
    let expanded = develop(&fixture);
    // x=0 maps to -1.4; constant RGB cannot reveal that SDK source-border clamping occurred.
    assert!(expanded.capture_state(0, 28).unwrap().repeated_border());
    assert!(!expanded.capture_state(0, 28).unwrap().eligible());
    assert!(expanded.capture_state(28, 28).unwrap().eligible());
}

#[test]
fn original_capture_limits_survive_gains_that_move_values_away_from_those_limits() {
    for (code, gain) in [(64, 4.), (4160, 0.5)] {
        let mut fixture = Fixture {
            marker: Some((30, 28, code)),
            ..Default::default()
        };
        fixture.lists[0].push(Opcode::gain(gain, 0));
        let source = develop(&fixture);
        let state = source.capture_state(24, 24).unwrap();
        assert!(if code == 64 {
            state.original_black()
        } else {
            state.original_white()
        });
        assert!(!state.eligible());
        assert_eq!(source.original_cfa()[28 * 64 + 30], code);
    }
}

#[test]
fn packed_fourteen_bit_and_uniform_black_patterns_keep_the_same_camera_basis() {
    let base = develop(&Fixture::default());
    let mut fixture = Fixture {
        bits: 14,
        ..Default::default()
    };
    fixture
        .extra
        .push((50713, 3, [2u16.to_le_bytes(), 2u16.to_le_bytes()].concat()));
    let black = [(64u32, 1u32); 4]
        .into_iter()
        .flat_map(|(n, d)| [n.to_le_bytes(), d.to_le_bytes()].concat())
        .collect();
    fixture.extra.push((50714, 5, black));
    let packed = develop(&fixture);
    assert_eq!(packed.original_cfa(), base.original_cfa());
    assert_eq!(packed.camera_rgb(), base.camera_rgb());
    assert_eq!(packed.interpretation_id(), base.interpretation_id());
    fixture.extra.last_mut().unwrap().2[..4].copy_from_slice(&65u32.to_le_bytes());
    assert!(Source::develop(fixture.write().path(), options(), &Cancellation::default()).is_err());
}

#[test]
fn required_bad_pixel_repair_retains_original_limits_and_neighbour_dependencies() {
    for (x, y, nx, ny, channel, expected) in [
        (30u32, 28u32, 32u32, 28u32, 0usize, 0.4375f32),
        (31, 28, 32, 29, 1, 0.53125),
    ] {
        for warped in [false, true] {
            let mut fixture = Fixture {
                marker: Some((x, y, 0)),
                ..Default::default()
            };
            fixture.lists[0].push(Opcode::bad_pixels(0, 2));
            if warped {
                fixture.lists[2].push(warp(true));
            }
            let file = fixture.write();
            let mut bytes = std::fs::read(file.path()).unwrap();
            let offset = bytes.len() - 64 * 64 * 2 + (ny * 64 + nx) as usize * 2;
            bytes[offset..offset + 2].copy_from_slice(&4160u16.to_le_bytes());
            std::fs::write(file.path(), bytes).unwrap();
            let source = Source::develop(file.path(), options(), &Cancellation::default()).unwrap();
            let (dx, dy) = ((x - 6) as usize, (y - 4) as usize);
            if !warped {
                assert_eq!(pixel(&source, dx, dy)[channel], expected);
            }
            let state = source.capture_state(dx as u32, dy as u32).unwrap();
            assert!(state.original_white() && state.original_black());
            assert!(!state.eligible());
            assert_eq!(source.original_cfa()[(y * 64 + x) as usize], 0);
            assert_eq!(source.original_cfa()[(ny * 64 + nx) as usize], 4160);
            assert!(source.capture_state(12, 12).unwrap().eligible());
        }
    }
}

#[test]
fn in_range_bad_pixel_sentinels_reject_rather_than_become_eligible_synthesized_capture() {
    let mut fixture = Fixture {
        marker: Some((30, 28, 1000)),
        ..Default::default()
    };
    let plain = fixture.write();
    let source = Source::develop(plain.path(), options(), &Cancellation::default()).unwrap();
    assert!(source.capture_state(24, 24).unwrap().eligible());
    fixture.lists[0].push(Opcode::bad_pixels(1000, 2));
    let repaired = fixture.write();
    assert!(Header::read(repaired.path(), options(), &Cancellation::default()).is_err());
    assert!(Source::develop(repaired.path(), options(), &Cancellation::default()).is_err());
}

#[test]
fn bad_pixel_marker_and_phase_reject_instead_of_sdk_integer_truncation() {
    for (constant, phase) in [(65536, 2), (0, 4)] {
        let mut fixture = Fixture::default();
        fixture.lists[0].push(Opcode::bad_pixels(constant, phase));
        let file = fixture.write();
        assert!(Header::read(file.path(), options(), &Cancellation::default()).is_err());
        assert!(Source::develop(file.path(), options(), &Cancellation::default()).is_err());
    }
}

#[test]
fn precision_floor_covers_integer_gain_rounding_before_float_linearization() {
    let mut fixture = Fixture::default();
    fixture.lists[0].push(Opcode::gain(1.1, 0));
    let file = fixture.write();
    let header = Header::read(file.path(), options(), &Cancellation::default()).unwrap();
    let source = Source::develop(file.path(), options(), &Cancellation::default()).unwrap();
    assert_eq!(header.input_floor, 2. / 4096.);
    assert_eq!(source.input_floor(), header.input_floor);
}

#[test]
fn header_inspection_matches_development_without_reading_sensor_payload() {
    let mut fixture = Fixture::default();
    fixture.lists[1].push(Opcode::gain(2., 1));
    fixture.lists[2].push(warp(true));
    let file = fixture.write();
    let cancel = Cancellation::default();
    let header = Header::read(file.path(), options(), &cancel).unwrap();
    let source = Source::develop(file.path(), options(), &cancel).unwrap();
    assert_eq!(header.interpretation_id, source.interpretation_id());
    assert_eq!(header.camera_to_xyz_d50, source.camera_to_xyz_d50());
    assert_eq!(header.input_floor, 4. / 4096.);
    let mut bytes = std::fs::read(file.path()).unwrap();
    bytes.truncate(bytes.len() - 64 * 64 * 2);
    std::fs::write(file.path(), bytes).unwrap();
    assert_eq!(
        Header::read(file.path(), options(), &cancel)
            .unwrap()
            .interpretation_id,
        header.interpretation_id
    );
    assert!(Source::develop(file.path(), options(), &cancel).is_err());
}

#[test]
fn completed_owner_can_be_destroyed_on_another_thread() {
    let source = develop(&Fixture::default());
    std::thread::spawn(move || {
        assert_eq!(pixel(&source, 20, 20), [0.25, 0.375, 0.5]);
        drop(source);
    })
    .join()
    .unwrap();
}

#[test]
fn optional_instructions_skip_but_required_or_future_instructions_never_fall_back() {
    let base = develop(&Fixture::default());
    for future in [false, true] {
        for optional in [false, true] {
            let mut fixture = Fixture::default();
            let mut opcode = if future {
                Opcode::gain(2., 1)
            } else {
                Opcode::unknown(optional)
            };
            opcode.flags = u32::from(optional);
            if future {
                opcode.version = 0x09000000;
            }
            fixture.lists[1].push(opcode);
            let file = fixture.write();
            let result = Source::develop(file.path(), options(), &Cancellation::default());
            if optional {
                let source = result.unwrap();
                assert_eq!(source.camera_rgb(), base.camera_rgb());
                assert_eq!(source.opcode_counts().1, [0, 1, 0]);
            } else {
                assert!(matches!(result, Err(Error::Rejected(_))));
            }
        }
    }
    // A known SDK operation in an unproven stage is outside this adapter's support too.
    let mut fixture = Fixture::default();
    fixture.lists[2].push(Opcode::gain(2., 2));
    assert!(matches!(
        Source::develop(fixture.write().path(), options(), &Cancellation::default()),
        Err(Error::Rejected(_))
    ));
}

#[test]
fn admission_rejects_unproved_sensor_domains_and_invalid_options() {
    let file = Fixture::default().write();
    for options in [
        Options {
            max_pixels: 4095,
            ..options()
        },
        Options {
            max_pixels: 0,
            ..options()
        },
        Options {
            fixed_wb: Some([0., 1., 1.]),
            ..options()
        },
        Options {
            fixed_wb: Some([f64::NAN, 1., 1.]),
            ..options()
        },
        Options {
            fixed_wb: Some([257., 1., 1.]),
            ..options()
        },
    ] {
        assert!(matches!(
            Source::develop(file.path(), options, &Cancellation::default()),
            Err(Error::Rejected(_))
        ));
    }
    for extra in [
        (258, 3, 32u16.to_le_bytes().to_vec()),
        (339, 3, 3u16.to_le_bytes().to_vec()),
        (50712, 3, [0u16.to_le_bytes(), 1u16.to_le_bytes()].concat()),
        (33422, 1, vec![0, 0, 1, 2]),
    ] {
        let fixture = Fixture {
            extra: vec![extra],
            ..Default::default()
        };
        assert!(matches!(
            Source::develop(fixture.write().path(), options(), &Cancellation::default()),
            Err(Error::Rejected(_))
        ));
    }
}

#[test]
fn interpretation_changes_with_program_and_sensor_layout_but_not_capture_values() {
    let base = develop(&Fixture::default());
    let marker = develop(&Fixture {
        marker: Some((30, 28, 4160)),
        ..Default::default()
    });
    assert_eq!(base.interpretation_id(), marker.interpretation_id());
    let bggr = develop(&Fixture {
        pattern: [2, 1, 1, 0],
        ..Default::default()
    });
    assert_eq!(base.camera_rgb(), bggr.camera_rgb());
    assert_ne!(base.interpretation_id(), bggr.interpretation_id());
    let mut gain = Fixture::default();
    gain.lists[1].push(Opcode::gain(2., 1));
    assert_ne!(base.interpretation_id(), develop(&gain).interpretation_id());
}

#[test]
fn cancellation_at_native_checkpoints_never_publishes_a_partial_owner() {
    struct Counter {
        calls: AtomicUsize,
        stop: usize,
    }
    extern "C" fn stop(context: *const c_void) -> c_int {
        // Borrowed synchronously by call_native; no panic can cross the C ABI.
        let counter = unsafe { &*context.cast::<Counter>() };
        c_int::from(counter.calls.fetch_add(1, Ordering::Relaxed) + 1 == counter.stop)
    }
    let mut fixture = Fixture::default();
    fixture.lists[2].push(warp(true));
    let file = fixture.write();
    let path = CString::new(file.path().to_str().unwrap()).unwrap();
    let baseline = Counter {
        calls: AtomicUsize::new(0),
        stop: usize::MAX,
    };
    let mut native = NativeOptions {
        threads: 1,
        max_pixels: 4096,
        wb: [1.; 3],
        cancelled: stop,
        cancel_context: (&baseline as *const Counter).cast(),
    };
    let source = Source::call_native(&path, &native).unwrap();
    let calls = baseline.calls.load(Ordering::Relaxed);
    assert!(calls > 100);
    for checkpoint in [1, 2, calls / 4, calls / 2, calls - 1, calls] {
        let counter = Counter {
            calls: AtomicUsize::new(0),
            stop: checkpoint,
        };
        native.cancel_context = (&counter as *const Counter).cast();
        assert!(matches!(
            Source::call_native(&path, &native),
            Err(Error::Cancelled)
        ));
        assert_eq!(counter.calls.load(Ordering::Relaxed), checkpoint);
    }
    assert_eq!(pixel(&source, 20, 20), [0.25, 0.375, 0.5]);
    let token = Cancellation::default();
    token.cancel();
    assert!(matches!(
        Source::develop(file.path(), options(), &token),
        Err(Error::Cancelled)
    ));
}
