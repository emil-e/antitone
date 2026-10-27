use super::*;

const N: usize = 96;

#[path = "gpu_film_emulation_cache_tests.rs"]
mod cache;
#[path = "gpu_film_emulation_spectrum_tests.rs"]
mod model;
#[path = "gpu_film_emulation_numerical_tests.rs"]
mod numerical;
#[path = "gpu_film_emulation_order_tests.rs"]
mod order;
#[path = "gpu_film_emulation_reference_tests.rs"]
mod reference;

#[derive(Clone, Copy)]
struct ProbeConfig {
    side: usize,
    scale: f64,
    footprint: f64,
    amount: f64,
    colour: f64,
    enabled: bool,
    impulse: bool,
}

impl Default for ProbeConfig {
    fn default() -> Self {
        Self {
            side: N,
            scale: 2.,
            footprint: 1.,
            amount: 0.055,
            colour: 0.,
            enabled: true,
            impulse: false,
        }
    }
}

// Monochrome mode cancels grain in green/red, isolating the image PSF.
fn probe_view(
    device: &wgpu::Device,
    _queue: &wgpu::Queue,
    renderer: &Arc<crate::renderer::GpuRenderer>,
    kernels: &Arc<ViewKernels>,
    [scale, footprint]: [f64; 2],
    enabled: bool,
    seed: u64,
) -> Arc<RenderedView> {
    configured_probe(
        device,
        renderer,
        kernels,
        ProbeConfig {
            scale,
            footprint,
            enabled,
            impulse: true,
            ..Default::default()
        },
        seed,
    )
}

fn configured_probe(
    device: &wgpu::Device,
    renderer: &Arc<crate::renderer::GpuRenderer>,
    kernels: &Arc<ViewKernels>,
    config: ProbeConfig,
    seed: u64,
) -> Arc<RenderedView> {
    let composition = Composition::new(
        crate::raw::Rect {
            x: 0,
            y: 0,
            width: 6000,
            height: 6000,
        },
        Default::default(),
        Default::default(),
    )
    .unwrap();
    let frame = FrameRaster::new(
        composition,
        FramePoint([1000., 1000.]),
        FramePoint([1000. + config.side as f64 * config.footprint; 2]),
        [config.side as u32; 2],
    )
    .unwrap();
    let mut values = vec![[0.18_f32, 0.18, 0.18, 8.]; config.side * config.side];
    if config.impulse {
        values[(config.side / 2) * config.side + config.side / 2][1] += 0.18;
    }
    let source = Arc::new(ViewSource {
        spec: ViewSourceSpec::new(seed, frame, InputSettings::Digital, DMat3::IDENTITY, None)
            .unwrap(),
        renderer: renderer.clone(),
        kernels: kernels.clone(),
        pixels: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("flat field and impulse"),
            contents: bytemuck::cast_slice(&values),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        }),
    });
    let snapshot = AdjustmentLayerSnapshot::new(vec![]).unwrap();
    let grade = Arc::new(
        GpuPipeline::new(
            device,
            InputSettings::Digital,
            RollSettings::default(),
            snapshot.clone(),
            DMat3::IDENTITY,
            composition.area(),
        )
        .unwrap(),
    );
    let input = ProcessingInput::from(grade)
        .with_film_emulation(FilmEmulationSettings {
            enabled: config.enabled,
            size: config.scale * (100. / 6000.),
            amount: config.amount,
            colour: config.colour,
        })
        .with_film_seed(seed);
    RenderedView::new(device, source, &input, OutputTarget::Srgb, None).unwrap()
}

fn render_probe(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &Arc<crate::renderer::GpuRenderer>,
    kernels: &Arc<ViewKernels>,
    scale_and_footprint: [f64; 2],
    enabled: bool,
    seed: u64,
) -> Vec<[f32; 4]> {
    let rendered = probe_view(
        device,
        queue,
        renderer,
        kernels,
        scale_and_footprint,
        enabled,
        seed,
    );
    rendered.ensure_submitted(
        device,
        queue,
        &SubmissionGate::default(),
        &egui::Context::default(),
    );
    read(device, queue, &rendered.graded.as_ref().unwrap().pixels)
}

fn gradient_bandwidth(values: &[f64]) -> f64 {
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let mut energy = 0.;
    let mut gradients = 0.;
    for y in 0..N {
        for x in 0..N {
            let v = values[y * N + x];
            energy += (v - mean).powi(2);
            gradients += (v - values[y * N + (x + 1) % N]).powi(2)
                + (v - values[((y + 1) % N) * N + x]).powi(2);
        }
    }
    (gradients / energy).sqrt()
}

fn spectrum(values: &[f64], window: bool) -> Vec<(f64, f64)> {
    // Small independent separable DFT; no radix-size assumptions or production filter reuse.
    let weights: Vec<_> = (0..N)
        .map(|i| {
            if window {
                (std::f64::consts::PI * i as f64 / (N - 1) as f64)
                    .sin()
                    .powi(2)
            } else {
                1.
            }
        })
        .collect();
    let mean = if window {
        values.iter().sum::<f64>() / values.len() as f64
    } else {
        0.
    };
    let twiddle: Vec<_> = (0..N * N)
        .map(|i| {
            let phase = -std::f64::consts::TAU * (i / N * (i % N)) as f64 / N as f64;
            glam::DVec2::new(phase.cos(), phase.sin())
        })
        .collect();
    let mut rows = vec![glam::DVec2::ZERO; N * N];
    for y in 0..N {
        for k in 0..N {
            for x in 0..N {
                rows[y * N + k] +=
                    twiddle[k * N + x] * ((values[y * N + x] - mean) * weights[x] * weights[y]);
            }
        }
    }
    let mut energy = Vec::with_capacity(N * N);
    for ky in 0..N {
        for kx in 0..N {
            let mut re = 0.;
            let mut im = 0.;
            for y in 0..N {
                let v = rows[y * N + kx];
                let w = twiddle[ky * N + y];
                re += v.x * w.x - v.y * w.y;
                im += v.x * w.y + v.y * w.x;
            }
            let fx = kx.min(N - kx) as f64 / N as f64;
            let fy = ky.min(N - ky) as f64 / N as f64;
            energy.push(((fx * fx + fy * fy).sqrt(), re * re + im * im));
        }
    }
    energy
}

#[test]
fn diagnostic_dft_recovers_a_declared_sinusoid() {
    let cycles = 3.;
    let values: Vec<_> = (0..N * N)
        .map(|i| (std::f64::consts::TAU * cycles * (i % N) as f64 / N as f64).cos())
        .collect();
    let energy = spectrum(&values, false);
    let total = energy.iter().map(|(_, e)| e).sum::<f64>();
    let peaks = energy[3].1 + energy[N - 3].1;
    assert!((peaks / total - 1.).abs() < 1e-12);
    assert!((total / (N as f64).powi(4) - 0.5).abs() < 1e-12);
}

#[test]
fn film_emulation_gpu_bandwidth_and_zoom_scaling() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        for scale in [1., 2., 3.5] {
            let mut previous_variance = f64::INFINITY;
            for footprint in [0.5, 1., 2., 4.] {
                let pixels = render_probe(
                    &device,
                    &queue,
                    &renderer,
                    &kernels,
                    [scale, footprint],
                    true,
                    17,
                );
                assert!(pixels.iter().flatten().all(|v| v.is_finite()));
                assert!(
                    pixels.iter().all(|p| p[0] > 0.),
                    "nonpositive probe pixels: {:?}",
                    &pixels[..4]
                );
                let psf: Vec<_> = pixels.iter().map(|p| f64::from(p[1] / p[0] - 1.)).collect();
                let grain: Vec<_> = pixels
                    .iter()
                    .map(|p| (f64::from(p[0]) / 0.18).ln())
                    .collect();
                let sum = psf.iter().sum::<f64>();
                assert!((sum - 1.).abs() < 0.002, "PSF integral {sum}");
                let second_moment: f64 = psf
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        let dx = (i % N) as f64 - (N / 2) as f64;
                        v * dx * dx
                    })
                    .sum::<f64>()
                    / sum;
                let sigma_source = second_moment.max(0.).sqrt() * footprint;
                let image_bandwidth = gradient_bandwidth(&psf);
                let grain_bandwidth = gradient_bandwidth(&grain);
                let mean = grain.iter().sum::<f64>() / grain.len() as f64;
                let variance =
                    grain.iter().map(|g| (g - mean).powi(2)).sum::<f64>() / grain.len() as f64;
                eprintln!(
                    "film_probe L={scale} footprint={footprint} sigma_source={sigma_source:.4} image_bw={image_bandwidth:.4} grain_bw={grain_bandwidth:.4} log_variance={variance:.6}"
                );
                // Below half-pixel sigma this view-sampled image kernel no longer resolves
                // its source PSF. The continuous bandwidth ordering is tested in the model.
                if scale / footprint >= 0.5 {
                    assert!(
                        (sigma_source / scale - 1.).abs() < 0.08,
                        "source sigma changes with zoom: L={scale}, p={footprint}, sigma={sigma_source}"
                    );
                    assert!(
                        grain_bandwidth > image_bandwidth,
                        "grain is not sharper: L={scale}, p={footprint}"
                    );
                }
                let image_spectrum = spectrum(&psf, false);
                let grain_spectrum = spectrum(&grain, true);
                let tail = |spectrum: &[(f64, f64)], cutoff: f64| {
                    spectrum
                        .iter()
                        .filter(|(f, _)| *f >= cutoff)
                        .map(|(_, e)| e)
                        .sum::<f64>()
                        / spectrum.iter().map(|(_, e)| e).sum::<f64>()
                };
                for source_cutoff in [0.1, 0.2, 0.3].map(|f| f / scale) {
                    let cutoff = source_cutoff * footprint;
                    if cutoff > 0.4 || scale / footprint < 0.5 {
                        continue;
                    }
                    let image_tail = tail(&image_spectrum, cutoff);
                    let grain_tail = tail(&grain_spectrum, cutoff);
                    assert!(
                        grain_tail > image_tail,
                        "spectral ordering at L={scale}, p={footprint}, f={source_cutoff}: grain={grain_tail}, image={image_tail}"
                    );
                }
                if footprint > 1. {
                    assert!(
                        variance < previous_variance,
                        "reduction must lose grain variance"
                    );
                }
                // Native RMS uses large source ensembles in the model test; this fixed
                // view covers progressively smaller source regions when magnifying.
                previous_variance = variance;
            }
        }
        let clean = render_probe(&device, &queue, &renderer, &kernels, [2., 1.], false, 17);
        assert!(clean.iter().enumerate().all(|(i, p)| {
            let green = if i == (N / 2) * N + N / 2 { 0.36 } else { 0.18 };
            (p[0] - 0.18).abs() < 1e-6 && (p[1] - green).abs() < 1e-6
        }));
    });
}
