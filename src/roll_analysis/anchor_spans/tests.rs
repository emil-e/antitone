use super::*;

#[test]
fn anchor_spans_set_proportional_channel_gammas_without_moving_the_anchors() {
    let spans = DVec3::new(0.8, 1., 1.25);
    let base = DVec3::new(0.24, 0.36, 0.48);
    let dense = base * (-spans).exp();
    let film = [base; 2048];
    let picture = [dense; 1024];
    let frames = [FrameSamples {
        camera_to_ap1: DMat3::IDENTITY,
        film: &film,
        picture: &picture,
    }];
    let result = estimate_anchor_spans(
        &frames,
        NegativeInput::from(Calibration {
            dmin: base,
            dmax: dense,
        })
        .into(),
        &mut model::ScalarReference,
        &mut || false,
    )
    .unwrap()
    .unwrap();
    let negative = result.input.negative_input().unwrap();
    assert!((negative.calibration.dmin - base).abs().max_element() < 1e-12);
    assert!((negative.calibration.dmax - dense).abs().max_element() < 1e-12);
    assert_eq!(negative.gamma, 0.60);
    assert!(
        (negative.channel_gammas().unwrap() - spans * 0.60)
            .abs()
            .max_element()
            < 1e-12
    );
    assert!((result.roll.exposure - 0.18_f64.log2()).abs() < 1e-10);
}

#[test]
fn isolated_black_pixels_do_not_block_calibration_but_a_black_anchor_does() {
    let base = DVec3::splat(0.5);
    let dense = DVec3::splat(0.05);
    let mut film = [base; 1024];
    film[0] = DVec3::ZERO;
    let mut picture = [dense; 1024];
    picture[0] = DVec3::ZERO;
    let estimate = |picture: &[DVec3]| {
        estimate_anchor_spans(
            &[FrameSamples {
                camera_to_ap1: DMat3::IDENTITY,
                film: &film,
                picture,
            }],
            NegativeInput::from(Calibration {
                dmin: base,
                dmax: dense,
            })
            .into(),
            &mut model::ScalarReference,
            &mut || false,
        )
    };

    let proposal = estimate(&picture).unwrap().unwrap();
    let calibration = proposal.input.calibration().unwrap();
    assert!((calibration.dmin - base).abs().max_element() < 1e-12);
    assert!((calibration.dmax - dense).abs().max_element() < 1e-12);

    picture[..8].fill(DVec3::ZERO);
    assert!(matches!(
        estimate(&picture),
        Err(Error::InvalidCalibration { .. })
    ));
}

#[test]
fn unrepresentable_anchor_spans_keep_the_incoming_balance_without_clamping() {
    let base = DVec3::splat(0.8);
    let dense = base * (-DVec3::new(0.02, 0.2, 2.)).exp();
    let film = [base; 2048];
    let picture = [dense; 1024];
    let frames = [FrameSamples {
        camera_to_ap1: DMat3::IDENTITY,
        film: &film,
        picture: &picture,
    }];
    let mut input = NegativeInput::from(Calibration {
        dmin: base,
        dmax: dense,
    });
    input.balance = Balance {
        warmth: 0.25,
        tint: 0.05,
    };
    let result = estimate_anchor_spans(
        &frames,
        input.into(),
        &mut model::ScalarReference,
        &mut || false,
    )
    .unwrap()
    .unwrap();
    assert_eq!(result.input.balance(), input.balance);
}
