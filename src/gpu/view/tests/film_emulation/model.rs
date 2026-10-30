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
                    [
                        2. / crate::film_emulation::GRAIN_SCALE,
                        2. * footprint_per_scale,
                    ],
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
fn film_emulation_gpu_grain_matches_at_fixed_frame_sampling_across_source_resolutions() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let mut maximum_error = 0_f64;
        let mut maximum_binary_error = 0_f64;
        for output in [512, 1024] {
            for size in [
                crate::film_emulation::MIN_SIZE,
                crate::film_emulation::DEFAULT_SIZE,
                crate::film_emulation::MAX_SIZE,
            ] {
                for colour in [0., 0.5, 1.] {
                    for seed in [17, 1234] {
                        let mut reference: Option<Vec<[f32; 4]>> = None;
                        for native in [2048, 3000, 4096, 8192] {
                            // Same interior frame region, seed, scale fraction and sample centres.
                            // Flat inputs isolate grain from the preview MTF approximation.
                            let config = ProbeConfig {
                                side: 64,
                                frame_extent: native,
                                origin: [f64::from(native) / 4.; 2],
                                scale: size / 100. * f64::from(native),
                                footprint: f64::from(native) / f64::from(output),
                                colour,
                                ..Default::default()
                            };
                            let view = configured_probe(&device, &renderer, &kernels, config, seed);
                            view.ensure_submitted(
                                &device,
                                &queue,
                                &SubmissionGate::default(),
                                &egui::Context::default(),
                            );
                            let pixels =
                                read(&device, &queue, &view.graded.as_ref().unwrap().pixels);
                            assert_eq!(pixels.len(), config.side * config.side);
                            assert!(pixels.iter().flatten().all(|p| p.is_finite()));
                            if let Some(expected) = &reference {
                                for (actual, expected) in pixels.iter().zip(expected) {
                                    for c in 0..3 {
                                        let error = (f64::from(actual[c]) / f64::from(expected[c])
                                            - 1.)
                                            .abs();
                                        maximum_error = maximum_error.max(error);
                                        if native.is_power_of_two() {
                                            maximum_binary_error = maximum_binary_error.max(error);
                                        }
                                    }
                                    assert_eq!(actual[3], expected[3]);
                                }
                            } else {
                                let log: Vec<_> =
                                    pixels.iter().map(|p| f64::from(p[1]).ln()).collect();
                                let mean = log.iter().sum::<f64>() / log.len() as f64;
                                let rms = (log.iter().map(|g| (g - mean).powi(2)).sum::<f64>()
                                    / log.len() as f64)
                                    .sqrt();
                                assert!(
                                    rms > 0.0001,
                                    "a bypassed grain stage cannot establish invariance"
                                );
                                reference = Some(pixels);
                            }
                        }
                    }
                }
            }
        }
        // Non-binary rescaling quantizes the f32 lattice phase differently. Its budget is
        // 256 relative f32 epsilons; power-of-two rescaling should need no phase allowance.
        eprintln!(
            "grain frame invariance: maximum relative error={maximum_error}, binary={maximum_binary_error}"
        );
        assert!(maximum_error < 256. * f64::from(f32::EPSILON));
        assert!(maximum_binary_error < 0.000005);
    });
}

#[test]
fn film_emulation_gpu_amount_is_preintegration_rms_with_sampling_loss() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        // Independent Gaussian-spectrum/aperture integrals, normalized at zero aperture.
        for (scale, expected_rms) in [
            (0.3, 0.022598097992742462),
            (1., 0.04206214641741791),
            (2., 0.04991842747183426),
            (3.5, 0.05306786234056999),
        ] {
            let mut variance = 0.;
            let mut mean_light = 0.;
            for seed in [17, 77, 1234] {
                let view = configured_probe(
                    &device,
                    &renderer,
                    &kernels,
                    ProbeConfig {
                        side: 512,
                        frame_extent: 4096,
                        // This frame admits the authored field scales within the Size bounds.
                        scale: scale / crate::film_emulation::GRAIN_SCALE,
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
            eprintln!(
                "native grain L={scale}: log RMS={rms}, expected={expected_rms}, mean multiplier={mean_light}"
            );
            // Finite patches, three declared seeds, and at most 1% omitted power.
            assert!((rms / expected_rms - 1.).abs() < 0.05);
            assert!((mean_light - 1.).abs() < 0.003);
        }
    });
}
