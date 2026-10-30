use glam::{DMat3, DVec2, DVec3};

use crate::{
    Error,
    color::{AP1_TO_XYZ, Balance, ap1_to_cones, opponent_coordinates},
    processing::NegativeInput,
    roll_analysis::{ForwardModel, model::SampleBatch},
};

type Bands = Vec<[Vec<DVec3>; 3]>;
const NONPOSITIVE_CONES: &str = "tonal band mean has nonpositive Bradford responses";

fn bands(
    negative: NegativeInput,
    matrix: DMat3,
    points: &[Vec<DVec3>],
    model: &mut impl ForwardModel,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<Option<Bands>, Error> {
    let batch = SampleBatch::new(points.iter().map(Vec::as_slice));
    let mut ranked: Vec<Vec<(usize, f64)>> = points.iter().map(|_| Vec::new()).collect();
    if !batch.read(model, negative, cancelled, |frame, sample, positive| {
        ranked[frame].push((sample, (AP1_TO_XYZ * (matrix * positive)).y));
        Ok(())
    })? {
        return Ok(None);
    }

    let mut result = Vec::with_capacity(points.len());
    for (frame, mut ranked) in points.iter().zip(ranked) {
        if cancelled() {
            return Ok(None);
        }
        ranked.sort_unstable_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        let groups = [(0.01, 0.33), (0.33, 0.67), (0.67, 0.99)].map(|(low, high)| {
            ranked[(low * frame.len() as f64).floor() as usize
                ..(high * frame.len() as f64).floor() as usize]
                .iter()
                .map(|&(i, _)| frame[i])
                .collect::<Vec<_>>()
        });
        if groups.iter().any(Vec::is_empty) {
            return Err(Error::UnusableSample("empty gamma tonal band"));
        }
        result.push(groups);
    }
    Ok(Some(result))
}

fn drift(means: [DVec3; 3]) -> Result<[DVec2; 2], Error> {
    let mut z = [DVec2::ZERO; 3];
    for (i, mean) in means.into_iter().enumerate() {
        let cones = ap1_to_cones() * mean;
        if !cones.is_finite() {
            return Err(Error::NumericalOverflow);
        }
        if cones.min_element() <= 0. {
            return Err(Error::UnusableSample(NONPOSITIVE_CONES));
        }
        z[i] = opponent_coordinates(cones.ln());
    }
    Ok([z[0] - z[1], z[2] - z[1]])
}

#[cfg(test)]
fn evaluate(
    negative: NegativeInput,
    matrix: DMat3,
    bands: &Bands,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<Option<f64>, Error> {
    evaluate_with_model(
        negative,
        matrix,
        bands,
        &mut super::model::ScalarReference,
        cancelled,
    )
}

fn evaluate_with_model(
    negative: NegativeInput,
    matrix: DMat3,
    bands: &Bands,
    model: &mut impl ForwardModel,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<Option<f64>, Error> {
    let batch = SampleBatch::new(
        bands
            .iter()
            .flat_map(|frame| frame.iter().map(Vec::as_slice)),
    );
    let mut means = vec![DVec3::ZERO; batch.groups.len()];
    if !batch.read(model, negative, cancelled, |group, _, positive| {
        means[group] += positive;
        Ok(())
    })? {
        return Ok(None);
    }
    for (mean, group) in means.iter_mut().zip(&batch.groups) {
        if group.is_empty() {
            return Err(Error::UnusableSample("empty patch"));
        }
        *mean /= group.len() as f64;
        if !mean.is_finite() {
            return Err(Error::NumericalOverflow);
        }
        *mean = matrix * *mean;
    }

    let mut total = [DVec2::ZERO; 2];
    for &frame in means.as_chunks::<3>().0 {
        if cancelled() {
            return Ok(None);
        }

        let d = drift(frame)?;
        for i in 0..2 {
            total[i] += d[i];
        }
    }
    // Shared drift, not a penalty forcing every photograph's bands to be gray.
    let score = total
        .iter()
        .map(|v| (v / bands.len() as f64).length_squared())
        .sum::<f64>();
    if !score.is_finite() {
        return Err(Error::NumericalOverflow);
    }
    Ok(Some(score))
}

fn search(
    negative: NegativeInput,
    matrix: DMat3,
    bands: &Bands,
    model: &mut impl ForwardModel,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<Option<(Balance, f64)>, Error> {
    let mut best = negative;
    let Some(mut score) = evaluate_with_model(best, matrix, bands, model, cancelled)? else {
        return Ok(None);
    };
    let mut step = 0.125;
    let mut evaluations = 1;
    while step > 1e-7 {
        let base = best;
        let mut improved = false;
        for dx in [-1., 0., 1.] {
            for dy in [-1., 0., 1.] {
                if cancelled() {
                    return Ok(None);
                }
                if dx == 0. && dy == 0. {
                    continue;
                }
                let balance = Balance {
                    warmth: base.balance.warmth + step * dx,
                    tint: base.balance.tint + step * dy,
                };
                if balance.warmth.abs().max(balance.tint.abs()) > 1. {
                    continue;
                }
                evaluations += 1;
                if evaluations > 4096 {
                    return Err(Error::UnusableSample("gamma drift search did not converge"));
                }
                let candidate = NegativeInput { balance, ..base };
                let value = match evaluate_with_model(candidate, matrix, bands, model, cancelled) {
                    Ok(Some(value)) => value,
                    Ok(None) => return Ok(None),
                    Err(Error::UnusableSample(NONPOSITIVE_CONES)) => continue,
                    Err(error) => return Err(error),
                };
                if value < score {
                    score = value;
                    best = candidate;
                    improved = true;
                }
            }
        }
        if !improved {
            step *= 0.5;
        }
    }
    Ok(Some((best.balance, score)))
}

#[cfg(test)]
fn fit(
    negative: NegativeInput,
    matrix: DMat3,
    points: &[Vec<DVec3>],
    cancelled: &mut impl FnMut() -> bool,
) -> Result<Option<Balance>, Error> {
    fit_with_model(
        negative,
        matrix,
        points,
        &mut super::model::ScalarReference,
        cancelled,
    )
}

pub(super) fn fit_with_model(
    negative: NegativeInput,
    matrix: DMat3,
    points: &[Vec<DVec3>],
    model: &mut impl ForwardModel,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<Option<Balance>, Error> {
    let initial = NegativeInput {
        balance: Balance::default(),
        ..negative
    };
    let Some(bands) = bands(initial, matrix, points, model, cancelled)? else {
        return Ok(None);
    };
    let spans = (negative.calibration.dmin / negative.calibration.dmax).ln();
    let xy = opponent_coordinates((spans / (spans.x * spans.y * spans.z).cbrt()).ln());
    let seeds = [
        negative.balance,
        Balance::default(),
        Balance {
            warmth: xy.x,
            tint: xy.y,
        },
    ];
    let mut winner = Balance::default();
    let mut score = f64::INFINITY;
    for seed in seeds {
        if seed.validate().is_err() {
            continue;
        }
        let (candidate, value) = match search(
            NegativeInput {
                balance: seed,
                ..negative
            },
            matrix,
            &bands,
            model,
            cancelled,
        ) {
            Ok(Some(result)) => result,
            Ok(None) => return Ok(None),
            Err(Error::UnusableSample(NONPOSITIVE_CONES)) => continue,
            Err(error) => return Err(error),
        };
        if value < score {
            winner = candidate;
            score = value;
        }
    }
    if !score.is_finite() {
        return Err(Error::UnusableSample("no usable gamma drift search start"));
    }
    Ok(Some(winner))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        color::wb_matrix,
        processing::{Calibration, DEFAULT_GAMMA, NegativeInput, Response},
    };

    fn forward(cal: Calibration, negative: NegativeInput, positive: DVec3) -> DVec3 {
        let (toe, shoulder) = negative.response.shape().unwrap();
        let gammas = negative.channel_gammas().unwrap();
        DVec3::from_array(std::array::from_fn(|c| {
            let span = (cal.dmin[c] / cal.dmax[c]).ln();
            let anchor = toe * span / (gammas[c] - shoulder * span / std::f64::consts::LN_10);
            let u = (positive[c].powf(toe) * anchor.exp_m1()).ln_1p();
            let density = gammas[c] * u / (toe + shoulder * u / std::f64::consts::LN_10);
            cal.dmin[c] * (-density).exp()
        }))
    }

    #[test]
    fn known_curves_recover_with_different_frame_colours_and_old_grades() {
        let cal = Calibration {
            dmin: DVec3::new(0.14, 0.098, 0.094),
            dmax: DVec3::new(0.017, 0.0052, 0.0029),
            input_floor: 1. / 65535.,
        };
        let truth = NegativeInput {
            calibration: cal,
            response: Response::film(),
            gamma: DEFAULT_GAMMA,
            balance: Balance {
                warmth: -0.35,
                tint: 0.09,
            },
        };
        let matrix = DMat3::from_cols(
            DVec3::new(1.1, 0.03, -0.07),
            DVec3::new(0.2, 0.8, -0.1),
            DVec3::new(-0.03, 0.08, 1.3),
        );
        let points = [
            Balance::default(),
            Balance {
                warmth: 0.2,
                tint: -0.1,
            },
            Balance {
                warmth: -0.15,
                tint: 0.1,
            },
        ]
        .map(|wb| {
            let direction = matrix.inverse() * wb_matrix(wb).unwrap() * DVec3::ONE;
            (0..90)
                .map(|i| {
                    forward(
                        cal,
                        truth,
                        direction * 2_f64.powf(-7. + i as f64 / 89. * 6.8),
                    )
                })
                .collect::<Vec<_>>()
        });
        for current in [
            NegativeInput {
                balance: Balance::default(),
                ..truth
            },
            NegativeInput {
                balance: Balance {
                    warmth: 0.3,
                    tint: -0.2,
                },
                ..truth
            },
        ] {
            let fitted = fit(current, matrix, &points, &mut || false)
                .unwrap()
                .unwrap();
            assert!((fitted.warmth - truth.balance.warmth).abs() < 2e-6);
            assert!((fitted.tint - truth.balance.tint).abs() < 2e-6);
        }
    }

    #[test]
    fn out_of_range_span_start_does_not_abort_usable_current_and_zero_starts() {
        let cal = Calibration {
            dmin: DVec3::splat(0.5),
            dmax: DVec3::new(0.1, 0.001, 0.00002),
            input_floor: 1. / 65535.,
        };
        let spans = (cal.dmin / cal.dmax).ln();
        let xy = opponent_coordinates((spans / (spans.x * spans.y * spans.z).cbrt()).ln());
        assert!(xy.abs().max_element() > 1.);
        let negative = NegativeInput {
            calibration: cal,
            response: Response::film(),
            gamma: DEFAULT_GAMMA,
            balance: Balance::default(),
        };
        let points = vec![
            (0..90)
                .map(|i| forward(cal, negative, DVec3::splat(0.1 + 0.8 * i as f64 / 89.)))
                .collect(),
        ];
        let fitted = fit(negative, DMat3::IDENTITY, &points, &mut || false)
            .unwrap()
            .unwrap();
        assert!(fitted.warmth.abs().max(fitted.tint.abs()) < 2e-6);
        assert!(matches!(
            fit(negative, -DMat3::IDENTITY, &points, &mut || false),
            Err(Error::UnusableSample("no usable gamma drift search start"))
        ));
    }

    #[test]
    fn opposite_scene_drifts_cancel_before_squaring_instead_of_forcing_each_frame_gray() {
        let cal = Calibration {
            dmin: DVec3::new(0.14, 0.098, 0.094),
            dmax: DVec3::new(0.017, 0.0052, 0.0029),
            input_floor: 1. / 65535.,
        };
        let negative = NegativeInput {
            calibration: cal,
            response: Response::film(),
            gamma: DEFAULT_GAMMA,
            balance: Balance::default(),
        };
        let groups: Bands = [1., -1.]
            .into_iter()
            .map(|sign| {
                [(0.1, sign), (0.2, 0.), (0.5, -sign)].map(|(level, direction)| {
                    let mean = wb_matrix(Balance {
                        warmth: 0.2 * direction,
                        tint: -0.1 * direction,
                    })
                    .unwrap()
                        * DVec3::splat(level);
                    vec![forward(cal, negative, mean)]
                })
            })
            .collect();
        assert!(
            evaluate(negative, DMat3::IDENTITY, &groups, &mut || false)
                .unwrap()
                .unwrap()
                < 1e-24
        );
        for frame in groups {
            assert!(
                evaluate(negative, DMat3::IDENTITY, &vec![frame], &mut || false)
                    .unwrap()
                    .unwrap()
                    > 0.01
            );
        }
    }

    #[test]
    fn tonal_differences_ignore_constant_wb_and_exposure_and_reject_undefined_cones() {
        let means = [
            DVec3::new(0.03, 0.02, 0.018),
            DVec3::new(0.2, 0.18, 0.13),
            DVec3::new(0.8, 0.9, 1.1),
        ];
        let reference = drift(means).unwrap();
        for wb in [
            Balance {
                warmth: 0.6,
                tint: -0.3,
            },
            Balance {
                warmth: -0.5,
                tint: 0.4,
            },
        ] {
            let matrix = wb_matrix(wb).unwrap();
            let shifted = drift(means.map(|v| matrix * v * 8.)).unwrap();
            for i in 0..2 {
                assert!((reference[i] - shifted[i]).length() < 1e-12);
            }
        }
        assert!(drift([DVec3::ZERO; 3]).is_err());
    }
}
