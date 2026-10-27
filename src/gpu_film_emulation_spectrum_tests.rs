use super::*;

fn correlation(values: &[f64], dx: usize, dy: usize) -> f64 {
    let (mut a, mut b, mut aa, mut bb, mut ab, mut count) = (0., 0., 0., 0., 0., 0.);
    for y in 0..N - dy {
        for x in 0..N - dx {
            let p = values[y * N + x];
            let q = values[(y + dy) * N + x + dx];
            a += p;
            b += q;
            aa += p * p;
            bb += q * q;
            ab += p * q;
            count += 1.;
        }
    }
    (ab - a * b / count) / ((aa - a * a / count) * (bb - b * b / count)).sqrt()
}

// Analytic Gaussian covariance integral, sampled with the declared pixel footprint.
// Folding accounts for sampling aliases; no procedural lattice or GPU filter code is reused.
// Separate scan-envelope assertions protect the chosen model's photographic direction.
fn study_spectrum(footprint_per_scale: f64) -> Vec<f64> {
    (0..N * N)
        .map(|i| {
            let f = |k: usize| {
                if k <= N / 2 {
                    k as f64 / N as f64
                } else {
                    k as f64 / N as f64 - 1.
                }
            };
            let x = f(i % N);
            let y = f(i / N);
            let mut energy = 0.;
            for iy in -4..=4 {
                for ix in -4..=4 {
                    let r2 = (x + ix as f64).powi(2) + (y + iy as f64).powi(2);
                    let f2 = r2 / footprint_per_scale.powi(2);
                    let psd: f64 = crate::film_emulation::GRAIN_BASIS
                        .iter()
                        .map(|basis| {
                            let variance = basis.sigma.powi(2);
                            let power: f64 = basis.factor[1].iter().map(|v| v * v).sum();
                            power
                                * 4.
                                * std::f64::consts::PI
                                * variance
                                * (-4. * std::f64::consts::PI.powi(2) * variance * f2).exp()
                        })
                        .sum();
                    energy += psd * (-std::f64::consts::TAU.powi(2) * r2 / 12.).exp();
                }
            }
            energy
        })
        .collect()
}

fn expected_correlation(energy: &[f64], dx: usize, dy: usize) -> f64 {
    energy
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let phase = std::f64::consts::TAU * ((i % N) * dx + (i / N) * dy) as f64 / N as f64;
            e * phase.cos()
        })
        .sum::<f64>()
        / energy.iter().sum::<f64>()
}

fn spectral_bands(energy: &[(f64, f64)]) -> [f64; 5] {
    let mut bands = [0.; 5];
    let total = energy.iter().map(|(_, e)| e).sum::<f64>();
    for &(f, e) in energy {
        bands[((f * 10.) as usize).min(4)] += e / total;
    }
    bands
}

#[test]
fn film_emulation_gpu_psd_and_autocorrelation_track_the_covariance_model() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        // These source/view footprints exercise both native sampling and reduction. The
        // independent target includes the same declared Gaussian pixel-integration convention.
        for footprint_per_scale in [0.5, 1., 2.] {
            let expected = study_spectrum(footprint_per_scale);
            let expected_bands = spectral_bands(
                &expected
                    .iter()
                    .enumerate()
                    .map(|(i, &e)| {
                        let x = (i % N).min(N - i % N) as f64 / N as f64;
                        let y = (i / N).min(N - i / N) as f64 / N as f64;
                        ((x * x + y * y).sqrt(), e)
                    })
                    .collect::<Vec<_>>(),
            );
            let lags = [
                (1, 0),
                (0, 1),
                (1, 1),
                (2, 0),
                (0, 2),
                (2, 2),
                (4, 0),
                (0, 4),
                (8, 0),
                (0, 8),
            ];
            let mut observed_bands = [0.; 5];
            let mut observed_correlations = [0.; 10];
            for seed in [17, 77, 1234] {
                let pixels = render_probe(
                    &device,
                    &queue,
                    &renderer,
                    &kernels,
                    [2., 2. * footprint_per_scale],
                    true,
                    seed,
                );
                let grain: Vec<_> = pixels
                    .iter()
                    .map(|p| (f64::from(p[0]) / 0.18).ln())
                    .collect();
                for (out, value) in observed_bands
                    .iter_mut()
                    .zip(spectral_bands(&spectrum(&grain, true)))
                {
                    *out += value / 3.;
                }
                for (i, (dx, dy)) in lags.into_iter().enumerate() {
                    observed_correlations[i] += correlation(&grain, dx, dy) / 3.;
                }
            }
            eprintln!(
                "grain_model p/L={footprint_per_scale} PSD expected={expected_bands:?} measured={observed_bands:?}"
            );
            for (observed, expected) in observed_bands.into_iter().zip(expected_bands) {
                // Four percentage points of normalized power allows finite-patch/Hann-window
                // uncertainty and the fit error, but catches the old low-frequency-heavy filter.
                assert!(
                    (observed - expected).abs() < 0.04,
                    "PSD p/L={footprint_per_scale}: observed={observed}, expected={expected}"
                );
            }
            for (i, (dx, dy)) in lags.into_iter().enumerate() {
                let expected = expected_correlation(&expected, dx, dy);
                let observed = observed_correlations[i];
                eprintln!(
                    "grain_model p/L={footprint_per_scale} lag=({dx},{dy}) ACF expected={expected:.5} measured={observed:.5}"
                );
                assert!(
                    (observed - expected).abs() < 0.04,
                    "ACF p/L={footprint_per_scale}, lag=({dx},{dy}): observed={observed}, expected={expected}"
                );
            }
        }
    });
}

#[test]
fn film_emulation_gpu_amount_is_native_source_pixel_rms() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        for scale in [0.3, 1., 2., 3.5] {
            let mut variance = 0.;
            let mut mean_light = 0.;
            for seed in [17, 77, 1234] {
                let view = configured_probe(
                    &device,
                    &renderer,
                    &kernels,
                    ProbeConfig {
                        side: 512,
                        scale,
                        ..Default::default()
                    },
                    seed,
                );
                view.ensure_submitted(
                    &device,
                    &queue,
                    &SubmissionGate::default(),
                    &egui::Context::default(),
                );
                let pixels = read(&device, &queue, &view.graded.as_ref().unwrap().pixels);
                let log: Vec<_> = pixels
                    .iter()
                    .map(|p| (f64::from(p[0]) / 0.18).ln())
                    .collect();
                let mean = log.iter().sum::<f64>() / log.len() as f64;
                variance +=
                    log.iter().map(|g| (g - mean).powi(2)).sum::<f64>() / log.len() as f64 / 3.;
                mean_light += pixels.iter().map(|p| f64::from(p[0]) / 0.18).sum::<f64>()
                    / pixels.len() as f64
                    / 3.;
            }
            let rms = variance.sqrt();
            eprintln!("native grain L={scale}: log RMS={rms}, mean multiplier={mean_light}");
            // Finite patches, three declared seeds, and at most 1% omitted power.
            assert!((rms / 0.055 - 1.).abs() < 0.05);
            assert!((mean_light - 1.).abs() < 0.003);
        }
    });
}
