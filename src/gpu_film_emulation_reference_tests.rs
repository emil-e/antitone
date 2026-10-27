use super::*;

fn render_configured(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &Arc<crate::renderer::GpuRenderer>,
    kernels: &Arc<ViewKernels>,
    config: ProbeConfig,
    seed: u64,
) -> Vec<[f32; 4]> {
    let view = configured_probe(device, renderer, kernels, config, seed);
    view.ensure_submitted(
        device,
        queue,
        &SubmissionGate::default(),
        &egui::Context::default(),
    );
    read(device, queue, &view.graded.as_ref().unwrap().pixels)
}

fn centred(values: &[f64]) -> (Vec<f64>, f64) {
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let residual: Vec<_> = values.iter().map(|v| v / mean - 1.).collect();
    let variance = residual.iter().map(|v| v * v).sum::<f64>() / values.len() as f64;
    (residual, variance.sqrt())
}

#[test]
fn film_emulation_gpu_matches_the_scan_cohorts_spatial_and_rgb_direction() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let side = 256;
        let mut metrics = [0.; 5];
        for seed in [17, 77, 1234] {
            let pixels = render_configured(
                &device,
                &queue,
                &renderer,
                &kernels,
                ProbeConfig {
                    side,
                    scale: 1.8,
                    colour: 0.5,
                    ..Default::default()
                },
                seed,
            );
            let signals: [Vec<f64>; 3] =
                std::array::from_fn(|c| pixels.iter().map(|p| p[c] as f64).collect());
            let (red, red_rms) = centred(&signals[0]);
            let (green, green_rms) = centred(&signals[1]);
            let (_, blue_rms) = centred(&signals[2]);
            let rg = red.iter().zip(&green).map(|(r, g)| r * g).sum::<f64>()
                / (side * side) as f64
                / red_rms
                / green_rms;
            let y: Vec<_> = pixels
                .iter()
                .map(|p| {
                    p[0] as f64 * 0.27222872 + p[1] as f64 * 0.67408177 + p[2] as f64 * 0.05368952
                })
                .collect();
            let (residual, rms) = centred(&y);
            let adjacent = residual
                .chunks_exact(side)
                .flat_map(|row| row.windows(2))
                .map(|p| p[0] * p[1])
                .sum::<f64>()
                / (side * (side - 1)) as f64
                / rms.powi(2);
            let reduced: Vec<_> = (0..side / 4)
                .flat_map(|y0| {
                    let y = &y;
                    (0..side / 4).map(move |x0| {
                        (0..4)
                            .flat_map(|dy| {
                                (0..4).map(move |dx| y[(y0 * 4 + dy) * side + x0 * 4 + dx])
                            })
                            .sum::<f64>()
                            / 16.
                    })
                })
                .collect();
            let (_, reduced_rms) = centred(&reduced);
            let crop: Vec<_> = (0..N)
                .flat_map(|y0| {
                    let y = &y;
                    (0..N).map(move |x0| y[((side - N) / 2 + y0) * side + (side - N) / 2 + x0])
                })
                .collect();
            let mut psd = spectrum(&crop, true);
            psd.sort_by(|a, b| a.0.total_cmp(&b.0));
            let half = psd.iter().map(|v| v.1).sum::<f64>() / 2.;
            let mut cumulative = 0.;
            let median = psd
                .iter()
                .find(|v| {
                    cumulative += v.1;
                    cumulative >= half
                })
                .unwrap()
                .0;
            for (sum, value) in metrics.iter_mut().zip([
                rg,
                blue_rms / green_rms,
                adjacent,
                reduced_rms / rms,
                median * 6000.,
            ]) {
                *sum += value / 3.;
            }
        }
        eprintln!(
            "scan-direction RG/blue-to-green/adjacent-Y/4x-box-retention/cycles-frame-height: {metrics:?}"
        );
        // Expanded empirical envelopes from the measured sky cohort, not stock specifications.
        // The old scalar/high-pass field fails correlation, spatial and reduction protections.
        assert!((0.5..0.8).contains(&metrics[0]));
        assert!((1.3..1.9).contains(&metrics[1]));
        assert!((0.48..0.72).contains(&metrics[2]));
        assert!((0.45..0.7).contains(&metrics[3]));
        assert!((750.0..1300.0).contains(&metrics[4]));
    });
}

#[test]
fn film_emulation_gpu_rgb_covariance_matches_the_cpu_gain_compensation() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let config = ProbeConfig {
            side: 256,
            scale: 1.8,
            colour: 0.5,
            ..Default::default()
        };
        for footprint in [1., 4.] {
            let mut covariance = [[0.; 3]; 3];
            for seed in [17, 77, 1234] {
                let pixels = render_configured(
                    &device,
                    &queue,
                    &renderer,
                    &kernels,
                    ProbeConfig {
                        footprint,
                        ..config
                    },
                    seed,
                );
                let signals: [Vec<f64>; 3] = std::array::from_fn(|c| {
                    pixels
                        .iter()
                        .map(|p| (p[c] as f64 / 0.18).ln() / config.amount)
                        .collect()
                });
                let means: [f64; 3] =
                    std::array::from_fn(|c| signals[c].iter().sum::<f64>() / pixels.len() as f64);
                for r in 0..3 {
                    for c in 0..3 {
                        covariance[r][c] += signals[r]
                            .iter()
                            .zip(&signals[c])
                            .map(|(a, b)| (a - means[r]) * (b - means[c]))
                            .sum::<f64>()
                            / pixels.len() as f64
                            / 3.;
                    }
                }
            }
            let settings = FilmEmulationSettings {
                enabled: true,
                size: config.scale * (100. / 6000.),
                colour: config.colour,
                ..Default::default()
            };
            let expected = settings
                .parameters(6000)
                .unwrap()
                .unwrap()
                .covariance(footprint);
            for r in 0..3 {
                for c in 0..3 {
                    let tolerance = 0.05 * (expected[r][r] * expected[c][c]).sqrt();
                    assert!(
                        (covariance[r][c] - expected[r][c]).abs() < tolerance,
                        "RGB covariance p={footprint}, ({r},{c}): measured={} expected={}",
                        covariance[r][c],
                        expected[r][c]
                    );
                }
            }
        }
    });
}

#[test]
fn film_emulation_gpu_preserves_ensemble_rgb_means_at_maximum_amount_and_colour() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        for colour in [0., 0.5, 1.] {
            let mut means = [0.; 3];
            for seed in [17, 77, 1234] {
                let pixels = render_configured(
                    &device,
                    &queue,
                    &renderer,
                    &kernels,
                    ProbeConfig {
                        side: 512,
                        amount: 0.2,
                        colour,
                        ..Default::default()
                    },
                    seed,
                );
                assert!(pixels.iter().flatten().all(|v| v.is_finite()));
                if colour == 0. {
                    assert!(pixels.iter().all(|p| p[0] == p[1] && p[1] == p[2]));
                }
                for c in 0..3 {
                    means[c] += pixels.iter().map(|p| p[c] as f64).sum::<f64>()
                        / pixels.len() as f64
                        / 0.18
                        / 3.;
                }
            }
            eprintln!("maximum Amount RGB mean multipliers, Colour={colour}: {means:?}");
            // Correlated grain permits finite-crop mean fluctuations; these large ensembles
            // still distinguish mean compensation from the several-percent lognormal bias.
            assert!(means.into_iter().all(|v| (v - 1.).abs() < 0.008));
        }
    });
}
