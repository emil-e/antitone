use super::*;

const N: usize = 96;

mod cache;
mod model;
mod numerical;
mod order;
mod reference;

#[derive(Clone, Copy)]
struct ProbeConfig {
    side: usize,
    frame_extent: u32,
    origin: [f64; 2],
    scale: f64,
    reserve_scale: Option<f64>,
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
            frame_extent: 6000,
            origin: [1000.; 2],
            scale: 2.,
            reserve_scale: None,
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
            width: config.frame_extent,
            height: config.frame_extent,
        },
        Default::default(),
        Default::default(),
    )
    .unwrap();
    let frame = FrameRaster::new(
        composition,
        FramePoint(config.origin),
        FramePoint(
            config
                .origin
                .map(|p| p + config.side as f64 * config.footprint),
        ),
        [config.side as u32; 2],
    )
    .unwrap();
    let settings = FilmEmulationSettings {
        enabled: config.enabled,
        size: config.scale * (100. / f64::from(config.frame_extent)),
        image_sigma: config.scale * (100. / f64::from(config.frame_extent)),
        amount: config.amount,
        colour: config.colour,
    };
    let acquisition = FilmEmulationSettings {
        size: config.reserve_scale.unwrap_or(config.scale)
            * (100. / f64::from(config.frame_extent)),
        image_sigma: config.reserve_scale.unwrap_or(config.scale)
            * (100. / f64::from(config.frame_extent)),
        ..settings
    };
    let spec = ViewSourceSpec::new(seed, frame, InputSettings::Digital, DMat3::IDENTITY, None)
        .unwrap()
        .with_pipeline(
            crate::pixel_pipeline::PixelPipeline::film(acquisition, [config.frame_extent; 2], seed)
                .unwrap(),
        )
        .unwrap();
    let size = spec.input_frame().unwrap().output;
    let offset = spec.plan.output_offset().unwrap();
    let mut values = vec![[0.18_f32, 0.18, 0.18, 8.]; (size[0] * size[1]) as usize];
    if config.impulse {
        // A destination-cell box impulse has unit area at every acquisition pitch.
        // Injecting a single finer cell would shrink its energy under reconstruction.
        let ratio: [f64; 2] =
            std::array::from_fn(|i| f64::from(spec.plan.output().output[i]) / config.side as f64);
        let low: [f64; 2] =
            std::array::from_fn(|i| f64::from(offset[i]) + (config.side / 2) as f64 * ratio[i]);
        let high: [f64; 2] = std::array::from_fn(|i| low[i] + ratio[i]);
        for y in low[1].floor() as u32..high[1].ceil() as u32 {
            for x in low[0].floor() as u32..high[0].ceil() as u32 {
                let mass = (high[0].min(f64::from(x + 1)) - low[0].max(f64::from(x))).max(0.)
                    * (high[1].min(f64::from(y + 1)) - low[1].max(f64::from(y))).max(0.);
                values[(y * size[0] + x) as usize][1] += 0.18 * mass as f32;
            }
        }
    }
    let source = Arc::new(ViewSource {
        spec,
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
        ProcessingInput::new(
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
        .with_film_emulation(settings)
        .with_frame_size(composition.crop().size)
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

// Independent one-dimensional box impulse → finite Gaussian → cell-area integral.
fn reconstructed_impulse(view: &RenderedView, sigma: f64) -> Vec<f64> {
    let spec = &view.graded.as_ref().unwrap().source.spec;
    let pitch = spec.plan.pitch()[0];
    let ratio = f64::from(spec.plan.output().output[0]) / N as f64;
    let offset = f64::from(spec.plan.output_offset().unwrap()[0]);
    let low = offset + (N / 2) as f64 * ratio;
    let high = low + ratio;
    let sigma = sigma / pitch;
    let radius = (4. * sigma).ceil() as i32;
    let mass: f64 = (-radius..=radius)
        .map(|i| (-0.5 * (f64::from(i) / sigma).powi(2)).exp())
        .sum();
    (0..N)
        .map(|x| {
            let start = offset + x as f64 * ratio;
            let end = start + ratio;
            (start.floor() as i32..end.ceil() as i32)
                .map(|cell| {
                    let area =
                        (end.min(f64::from(cell + 1)) - start.max(f64::from(cell))).max(0.) / ratio;
                    let blurred: f64 = (-radius..=radius)
                        .map(|tap| {
                            let source = f64::from(cell + tap);
                            let impulse = (high.min(source + 1.) - low.max(source)).max(0.);
                            impulse * (-0.5 * (f64::from(tap) / sigma).powi(2)).exp() / mass
                        })
                        .sum();
                    area * blurred
                })
                .sum()
        })
        .collect()
}

#[test]
fn film_emulation_gpu_bandwidth_and_zoom_scaling() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        for scale in [1., 2., 3.5] {
            let mut previous_variance = f64::INFINITY;
            for footprint in [0.5, 1., 2., 4.] {
                let view = probe_view(
                    &device,
                    &queue,
                    &renderer,
                    &kernels,
                    [scale, footprint],
                    true,
                    17,
                );
                view.ensure_submitted(
                    &device,
                    &queue,
                    &SubmissionGate::default(),
                    &egui::Context::default(),
                );
                let pixels = read(&device, &queue, &view.graded.as_ref().unwrap().pixels);
                let expected = reconstructed_impulse(&view, scale);
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
                // The source-cell and destination apertures contribute to measured width
                // during reduction. Check those composed losses against a scalar box/tap
                // integral instead of treating the destination second moment as Gaussian sigma.
                let expected_moment: f64 = expected
                    .iter()
                    .enumerate()
                    .map(|(x, v)| v * (x as f64 - (N / 2) as f64).powi(2))
                    .sum();
                assert!(
                    (second_moment - expected_moment).abs() < 0.0005 * expected_moment.max(1.),
                    "composed aperture moment L={scale} p={footprint}: {second_moment} vs {expected_moment}"
                );
                for (i, value) in psf.iter().enumerate() {
                    assert!(
                        (value - expected[i % N] * expected[i / N]).abs() < 3e-6,
                        "box/tap/area response L={scale} p={footprint} at {i}"
                    );
                }
                if footprint <= 1. {
                    assert!((sigma_source / scale - 1.).abs() < 0.08);
                }
                if scale / footprint >= 0.5 {
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
