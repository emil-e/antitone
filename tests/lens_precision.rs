#![cfg(all(
    feature = "raw",
    feature = "gpu",
    feature = "ocio-reference",
    target_os = "macos"
))]
use antitone::adjustment_layers::AdjustmentLayerSettings;

use antitone as api;
use api::{
    flat_field::FlatField,
    gpu::{GpuFlatField, ProcessingKernel, ProcessingParameters, ProcessingSource, SourceGeometry},
    lens::EmbeddedLensGeometry,
    processing::{CAMERA_INPUT_FLOOR, Calibration, RollSettings},
    raw::{RawFile, Rect},
    rendering::{CpuRenderer, OutputTarget},
    scan::Scan,
};
use glam::{DMat3, DVec3};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
#[path = "support/train_raw.rs"]
mod train_raw;

struct Dataset {
    label: String,
    size: [u32; 2],
    codes: Vec<[f32; 4]>,
    eligible: Vec<bool>,
    calibration: Option<Calibration>,
    matrix: DMat3,
    flat: bool,
    spatial: bool,
}

fn quantized(pixel: [f32; 4], fractional_bits: u32) -> [f32; 4] {
    let scale = (1u32 << fractional_bits) as f32;
    pixel.map(|v| (v * scale).round() / scale)
}

fn camera(p: [f32; 4]) -> DVec3 {
    DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64) / 65535.
}

fn sample(
    scan: &Scan,
    geometry: Option<&EmbeddedLensGeometry>,
    tile: bool,
    label: &str,
    calibration: Option<Calibration>,
) -> Dataset {
    let image = scan.source().camera_image().unwrap();
    let dimensions = geometry.map_or([image.width, image.height], EmbeddedLensGeometry::size);
    let size = if tile { [384, 384] } else { [256, 171] };
    let mut codes = Vec::new();
    let mut eligible = Vec::new();
    for y in 0..size[1] {
        for x in 0..size[0] {
            let position = if tile {
                [
                    dimensions[0] / 2 - size[0] / 2 + x,
                    dimensions[1] / 2 - size[1] / 2 + y,
                ]
            } else {
                [
                    ((2 * x + 1) as u64 * dimensions[0] as u64 / (2 * size[0]) as u64) as u32,
                    ((2 * y + 1) as u64 * dimensions[1] as u64 / (2 * size[1]) as u64) as u32,
                ]
            };
            let (pixel, flags) = if let Some(geometry) = geometry {
                let positions = geometry.sample_positions(position[0], position[1]).unwrap();
                let mut pixel = [0.; 4];
                for (c, p) in positions.iter().enumerate() {
                    let left = p.x.floor() as u32;
                    let top = p.y.floor() as u32;
                    let u = p.x - left as f64;
                    let v = p.y - top as f64;
                    let p00 = image.pixel_samples(left, top).unwrap()[c] as f64;
                    let p10 = image.pixel_samples(p.x.ceil() as u32, top).unwrap()[c] as f64;
                    let p01 = image.pixel_samples(left, p.y.ceil() as u32).unwrap()[c] as f64;
                    let p11 = image
                        .pixel_samples(p.x.ceil() as u32, p.y.ceil() as u32)
                        .unwrap()[c] as f64;
                    pixel[c] = ((p00 * (1. - u) + p10 * u) * (1. - v)
                        + (p01 * (1. - u) + p11 * u) * v) as f32;
                }
                // Diagnostic-only borrowed warps have no published mask. Use a conservative
                // native bounding support to exclude questionable capture data from the study.
                let left =
                    (positions.iter().map(|p| p.x.floor() as u32).min().unwrap()).saturating_sub(5);
                let top =
                    (positions.iter().map(|p| p.y.floor() as u32).min().unwrap()).saturating_sub(5);
                let right = (positions.iter().map(|p| p.x.ceil() as u32).max().unwrap() + 6)
                    .min(image.width);
                let bottom = (positions.iter().map(|p| p.y.ceil() as u32).max().unwrap() + 6)
                    .min(image.height);
                (
                    pixel,
                    scan.source()
                        .capture_eligibility(Rect::from([left, top, right - left, bottom - top]))
                        .unwrap(),
                )
            } else {
                (
                    image.pixel_samples(position[0], position[1]).unwrap(),
                    scan.source()
                        .capture_eligibility(Rect::from([position[0], position[1], 1, 1]))
                        .unwrap(),
                )
            };
            eligible.push(
                !flags.at_white
                    && !flags.below_black
                    && pixel[..3]
                        .iter()
                        .all(|&v| v as f64 > scan.effective_floor() * 65535.),
            );
            codes.push(pixel);
        }
    }
    Dataset {
        label: label.into(),
        size,
        codes,
        eligible,
        calibration,
        matrix: scan.camera_to_ap1,
        flat: false,
        spatial: false,
    }
}

fn synthetic(near_base: bool) -> Dataset {
    let size = [512, 128];
    let dmin = DVec3::new(30000.25, 24000.5, 18000.75);
    let dmax = DVec3::new(240., 192., 144.);
    let calibration = Calibration {
        dmin: dmin / 65535.,
        dmax: dmax / 65535.,
        input_floor: CAMERA_INPUT_FLOOR,
    };
    let mut codes = Vec::new();
    for y in 0..size[1] {
        for x in 0..size[0] {
            let mut p = [0.; 4];
            for c in 0..3 {
                let value = if near_base {
                    dmin[c] + (x as f64 / 511. - 0.5) * 16.
                } else {
                    (1.25f64.ln() + (dmin[c].ln() - 1.25f64.ln()) * x as f64 / 511.).exp()
                };
                // A bilinear blend of four declared integer neighbours, not arbitrary fractional data.
                let a = value.floor();
                let u = (x % 31) as f64 / 31.;
                let v = (y % 29) as f64 / 29.;
                p[c] = ((a * (1. - u) + (a + 1.) * u) * (1. - v)
                    + ((a + 1.) * (1. - u) + (a + 2.) * u) * v) as f32;
            }
            codes.push(p);
        }
    }
    Dataset {
        label: if near_base {
            "synthetic-near-base"
        } else {
            "synthetic-dense-through-base"
        }
        .into(),
        size,
        eligible: vec![true; codes.len()],
        codes,
        calibration: Some(calibration),
        matrix: DMat3::IDENTITY,
        flat: false,
        spatial: false,
    }
}

enum ProcessingCase {
    Source(ProcessingParameters),
    Layers(antitone::gpu::GpuPipeline),
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    kernel: ProcessingKernel,
}

impl Gpu {
    async fn new() -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        println!("precision adapter={:?}", adapter.get_info());
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let kernel = ProcessingKernel::new(&device).await.unwrap();
        Self {
            device,
            queue,
            kernel,
        }
    }

    fn validate(
        &self,
        source: &ProcessingSource,
        output: &wgpu::Buffer,
        parameters: &ProcessingCase,
        expected: &[DVec3],
    ) {
        self.time(source, output, parameters);
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("precision GPU verification"),
            size: output.size(),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(output, 0, &readback, 0, output.size());
        self.queue.submit([encoder.finish()]);
        let (send, receive) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |r| send.send(r).unwrap());
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        receive.recv().unwrap().unwrap();
        let mapped = readback.slice(..).get_mapped_range().unwrap();
        let actual: &[[f32; 4]] = bytemuck::cast_slice(&mapped);
        for i in (0..expected.len()).step_by(257) {
            for (c, value) in actual[i][..3].iter().enumerate() {
                assert!(
                    (*value as f64 - expected[i][c]).abs() <= 2e-5 + 2e-4 * expected[i][c].abs(),
                    "precision GPU pixel={i} channel={c}, actual={value} expected={}",
                    expected[i][c]
                );
            }
        }
    }

    fn time(
        &self,
        source: &ProcessingSource,
        output: &wgpu::Buffer,
        parameters: &ProcessingCase,
    ) -> f64 {
        let start = Instant::now();
        let mut encoder = self.device.create_command_encoder(&Default::default());
        match parameters {
            ProcessingCase::Source(parameters) => {
                self.kernel
                    .encode(&self.device, &mut encoder, source, output, *parameters)
            }
            ProcessingCase::Layers(grade) => self
                .kernel
                .encode_grade(&self.device, &mut encoder, source, output, grade)
                .unwrap(),
        }
        let submission = self.queue.submit([encoder.finish()]);
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .unwrap();
        start.elapsed().as_secs_f64() * 1000.
    }
}

fn ppm(path: &Path, size: [u32; 2], pixels: &[[f64; 3]]) {
    let mut file = fs::File::create(path).unwrap();
    write!(file, "P6\n{} {}\n255\n", size[0], size[1]).unwrap();
    for pixel in pixels {
        file.write_all(&pixel.map(|v| (v * 255.).round() as u8))
            .unwrap();
    }
}

fn benchmark(
    gpu: &Gpu,
    dataset: &Dataset,
    output: &wgpu::Buffer,
    parameters: &ProcessingCase,
    sources: &[(&str, &ProcessingSource)],
    exposure: f64,
) {
    for &(_, source) in sources {
        gpu.time(source, output, parameters);
    }
    let mut times = vec![Vec::new(); sources.len()];
    for round in 0..7 {
        for offset in 0..sources.len() {
            let i = (round + offset) % sources.len();
            times[i].push(gpu.time(sources[i].1, output, parameters));
        }
    }
    for ((name, _), values) in sources.iter().zip(&mut times) {
        values.sort_by(f64::total_cmp);
        println!(
            "precision gpu={} flat={} ev={exposure} format={name} samples=7 median_ms={:.6} excludes=upload_readback_rendering includes=encode_submit_completion warmed=true",
            dataset.label, dataset.flat, values[3]
        );
    }
}

fn report(dataset: &Dataset, gpu: &Gpu, output_dir: &Path, csv: &mut fs::File) {
    let target = OutputTarget::Srgb;
    let renderer = CpuRenderer::new(target).unwrap();
    let fits_rgb16 = dataset
        .codes
        .iter()
        .flatten()
        .all(|&v| (0. ..=65535.).contains(&v));
    let codes16: Vec<_> = dataset
        .codes
        .iter()
        .map(|p| p.map(|v| v.round() as u16))
        .collect();
    let mut float_source =
        ProcessingSource::new_continuous(&gpu.device, &dataset.codes, dataset.matrix).unwrap();
    let mut integer_source =
        fits_rgb16.then(|| ProcessingSource::new(&gpu.device, &codes16, dataset.matrix).unwrap());
    let rounded: Vec<_> = dataset.codes.iter().map(|&p| quantized(p, 0)).collect();
    let mut rounded_float_source =
        ProcessingSource::new_continuous(&gpu.device, &rounded, dataset.matrix).unwrap();
    let gain = if dataset.flat {
        DVec3::new(0.9, 1.1, 1.05)
    } else {
        DVec3::ONE
    };
    let nodes = if dataset.spatial {
        [
            DVec3::new(0.8, 1.1, 0.9),
            DVec3::new(1.2, 0.9, 1.1),
            DVec3::new(0.9, 0.85, 1.15),
            DVec3::new(1.1, 1.15, 0.85),
        ]
    } else {
        [gain; 4]
    };
    let maximum_gain = nodes
        .iter()
        .map(|node| node.max_element())
        .fold(1f64, f64::max);
    // The spatial field has unity gain at its centre, where the fixed reference anchors live.
    // The constant field separately controls scale invariance and corrected-path timing.
    let reference_gain = if dataset.spatial { DVec3::ONE } else { gain };
    let calibration = dataset.calibration.map(|cal| Calibration {
        dmin: cal.dmin * reference_gain,
        dmax: cal.dmax * reference_gain,
        input_floor: cal.input_floor * maximum_gain,
    });
    let gain_at = |index: usize| {
        if !dataset.spatial {
            return gain;
        }
        let u = (index % dataset.size[0] as usize) as f64 / dataset.size[0] as f64
            + 0.5 / dataset.size[0] as f64;
        let v = (index / dataset.size[0] as usize) as f64 / dataset.size[1] as f64
            + 0.5 / dataset.size[1] as f64;
        (nodes[0] * (1. - u) + nodes[1] * u) * (1. - v) + (nodes[2] * (1. - u) + nodes[3] * u) * v
    };
    if dataset.flat {
        let area = [0, 0, dataset.size[0], dataset.size[1]];
        let field = Arc::new(
            GpuFlatField::new(
                &gpu.device,
                Arc::new(FlatField::from_nodes(area, [2, 2], nodes.to_vec()).unwrap()),
            )
            .unwrap(),
        );
        let geometry = SourceGeometry::new(area, dataset.size).unwrap();
        for source in [&mut float_source, &mut rounded_float_source]
            .into_iter()
            .chain(integer_source.iter_mut())
        {
            source
                .set_flat_field(Some(field.clone()), geometry)
                .unwrap();
        }
    }
    let output = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("precision timing output"),
        size: dataset.codes.len() as u64 * 16,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let fractional = dataset
        .codes
        .iter()
        .filter(|p| p[..3].iter().any(|v| v.fract() != 0.))
        .count();
    let floor_changes = dataset
        .codes
        .iter()
        .filter(|p| p[..3].iter().any(|&v| v > 1. && v.round() <= 1.))
        .count();
    let white_changes = dataset
        .codes
        .iter()
        .filter(|p| p[..3].iter().any(|&v| v < 65535. && v.round() >= 65535.))
        .count();
    println!(
        "precision source={} pixels={} eligible={} fractional={} floor_crossings={} white_crossings={} fits_rgb16={fits_rgb16} gpu_source_bytes=f32:{},u16:{:?}",
        dataset.label,
        dataset.codes.len(),
        dataset.eligible.iter().filter(|v| **v).count(),
        fractional,
        floor_changes,
        white_changes,
        float_source.byte_len(),
        integer_source.as_ref().map(ProcessingSource::byte_len)
    );
    let exposures: &[f64] = if dataset.calibration.is_some() {
        &[0., 2., 4., 8.]
    } else {
        &[0., 2., 4.]
    };
    for &exposure in exposures {
        let roll = RollSettings::default();
        let photo = AdjustmentLayerSettings {
            exposure,
            ..Default::default()
        };
        let pipeline = calibration.map(|cal| {
            antitone::adjustment_layers::AdjustmentLayerSnapshot::new(vec![
                antitone::adjustment_layers::AdjustmentLayer::global(photo),
            ])
            .and_then(|layers| {
                antitone::adjustment_layers::AdjustmentLayerPipeline::new(
                    cal,
                    roll,
                    layers,
                    dataset.matrix,
                )
            })
            .unwrap()
        });
        let evaluate = |i, p| match &pipeline {
            Some(pipeline) => pipeline.apply(camera(p) * gain_at(i), [0, 0]).unwrap().rgb,
            None => dataset.matrix * camera(p) * exposure.exp2(),
        };
        let reference: Vec<_> = dataset
            .codes
            .iter()
            .enumerate()
            .map(|(i, &p)| evaluate(i, p))
            .collect();
        let reference_render = renderer.render(&reference).unwrap();
        let encoded = |rgb: glam::Vec3| rgb.to_array().map(|v| target.encode(v as f64).unwrap());
        let reference_encoded: Vec<_> = reference_render.into_iter().map(encoded).collect();
        if exposure == 0. || exposure == 8. {
            ppm(
                &output_dir.join(format!("{}-ev{exposure}-f32.ppm", dataset.label)),
                dataset.size,
                &reference_encoded,
            );
        }
        for bits in [0, 8] {
            let candidate: Vec<_> = dataset
                .codes
                .iter()
                .enumerate()
                .map(|(i, &p)| evaluate(i, quantized(p, bits)))
                .collect();
            let rendered = renderer.render(&candidate).unwrap();
            let encoded_candidate: Vec<_> = rendered.into_iter().map(encoded).collect();
            let mut errors = Vec::new();
            let mut ap1_max = 0f64;
            let mut camera_max = 0f64;
            let mut worst = (0usize, 0f64);
            let mut changed8 = 0usize;
            let mut changed8_max = 0f64;
            for (i, (&valid, p)) in dataset.eligible.iter().zip(&dataset.codes).enumerate() {
                if !valid {
                    continue;
                }
                ap1_max = ap1_max.max((candidate[i] - reference[i]).abs().max_element());
                let q = quantized(*p, bits);
                for c in 0..3 {
                    let error = (encoded_candidate[i][c] - reference_encoded[i][c]).abs() * 65535.;
                    errors.push(error);
                    if error > worst.1 {
                        worst = (i, error);
                    }
                    let quant8 = ((encoded_candidate[i][c] * 255.).round()
                        - (reference_encoded[i][c] * 255.).round())
                    .abs();
                    changed8 += usize::from(quant8 != 0.);
                    changed8_max = changed8_max.max(quant8);
                    camera_max = camera_max.max((p[c] - q[c]).abs() as f64);
                }
            }
            assert!(!errors.is_empty());
            errors.sort_by(f64::total_cmp);
            let max = *errors.last().unwrap();
            let p99 = errors[(errors.len() - 1) * 99 / 100];
            let p999 = errors[(errors.len() - 1) * 999 / 1000];
            let rms = (errors.iter().map(|v| v * v).sum::<f64>() / errors.len() as f64).sqrt();
            let above1 = errors.iter().filter(|&&v| v > 1.).count();
            let above8bit = errors.iter().filter(|&&v| v > 257.).count();
            let name = match (fits_rgb16, bits) {
                (true, 0) => "rgb16",
                (true, _) => "q16_8",
                (false, 0) => "whole-code",
                (false, _) => "code-q8",
            };
            println!(
                "precision image={} ev={exposure} storage={name} camera_max_code={camera_max:.9} ap1_max_abs={ap1_max:.9e} display_rgb16_rms={rms:.6} p99={p99:.6} p999={p999:.6} max={max:.6} above1={above1}/{} above_one_8bit={above8bit}",
                dataset.label,
                errors.len()
            );
            writeln!(csv, "{},{exposure},{name},{camera_max},{ap1_max},{rms},{p99},{p999},{max},{above1},{above8bit},{}", dataset.label, errors.len()).unwrap();
            println!(
                "precision outlier={} ev={exposure} storage={name} pixel={} input={:?} ap1_f32={:?} ap1_candidate={:?} encoded_f32={:?} encoded_candidate={:?} quantized8_changed={changed8}/{} quantized8_max_difference={changed8_max}",
                dataset.label,
                worst.0,
                dataset.codes[worst.0],
                reference[worst.0],
                candidate[worst.0],
                reference_encoded[worst.0],
                encoded_candidate[worst.0],
                errors.len()
            );
            if bits == 0 && (exposure == 0. || exposure == 8.) {
                ppm(
                    &output_dir.join(format!("{}-ev{exposure}-{name}.ppm", dataset.label)),
                    dataset.size,
                    &encoded_candidate,
                );
                let difference: Vec<_> = reference_encoded
                    .iter()
                    .zip(&encoded_candidate)
                    .map(|(a, b)| std::array::from_fn(|c| ((a[c] - b[c]).abs() * 32.).min(1.)))
                    .collect();
                ppm(
                    &output_dir.join(format!("{}-ev{exposure}-difference-x32.ppm", dataset.label)),
                    dataset.size,
                    &difference,
                );
            }
        }
        let parameters = match calibration {
            Some(cal) => antitone::adjustment_layers::AdjustmentLayerSnapshot::new(vec![
                antitone::adjustment_layers::AdjustmentLayer::global(photo),
            ])
            .and_then(|snapshot| {
                antitone::gpu::GpuPipeline::new(
                    &gpu.device,
                    cal,
                    roll,
                    snapshot,
                    dataset.matrix,
                    [0, 0, dataset.size[0], dataset.size[1]].into(),
                )
            })
            .map(ProcessingCase::Layers),
            None => ProcessingParameters::source_inspection(exposure).map(ProcessingCase::Source),
        };
        match parameters {
            Ok(parameters) => {
                let mut sources = vec![("f32-fractional", &float_source)];
                if let Some(integer) = &integer_source {
                    sources.push(("u16-packed", integer));
                }
                sources.push(("f32-rounded", &rounded_float_source));
                benchmark(gpu, dataset, &output, &parameters, &sources, exposure);
                gpu.validate(&float_source, &output, &parameters, &reference);
                let rounded_reference: Vec<_> = dataset
                    .codes
                    .iter()
                    .enumerate()
                    .map(|(i, &p)| evaluate(i, quantized(p, 0)))
                    .collect();
                if let Some(integer) = &integer_source {
                    gpu.validate(integer, &output, &parameters, &rounded_reference);
                }
                gpu.validate(
                    &rounded_float_source,
                    &output,
                    &parameters,
                    &rounded_reference,
                );
            }
            Err(error) => println!(
                "precision gpu={} ev={exposure} unsupported={error}",
                dataset.label
            ),
        }
    }
}

#[test]
fn rgb16_rounding_adds_a_half_code_bound_but_changes_fractional_samples() {
    let dataset = synthetic(false);
    let mut changed = 0;
    for p in dataset.codes {
        let q = quantized(p, 0);
        for c in 0..3 {
            assert!((p[c] - q[c]).abs() <= 0.5);
            changed += usize::from(p[c] != q[c]);
        }
    }
    assert!(changed > 0);
}

#[test]
#[ignore = "bounded read-only f32/RGB16 storage study; requires supplied RAW samples and OCIO; run in release"]
fn supplied_raw_storage_precision_study() {
    let root =
        PathBuf::from(std::env::var_os("ANTITONE_RAW_SAMPLES").expect("set ANTITONE_RAW_SAMPLES"));
    let output_dir = PathBuf::from(".cache/lens-precision");
    fs::create_dir_all(&output_dir).unwrap();
    let mut csv = fs::File::create(output_dir.join("metrics.csv")).unwrap();
    writeln!(csv, "dataset,ev,storage,camera_max_code,ap1_max_abs,display16_rms,display16_p99,display16_p999,display16_max,above1,above_one_8bit,channels").unwrap();
    let gpu = pollster::block_on(Gpu::new());
    for dataset in [synthetic(false), synthetic(true)] {
        report(&dataset, &gpu, &output_dir, &mut csv);
    }
    let profiles: Vec<_> = ["DSC00638.ARW", "DSC00632.ARW"]
        .into_iter()
        .map(|name| {
            let raw = RawFile::open(&root.join("non-scans").join(name)).unwrap();
            raw.metadata.vendor.sony_lens.profile().unwrap().unwrap()
        })
        .collect();
    let (train, calibration) = train_raw::load();
    println!(
        "precision real_film_calibration={calibration:?} wb={:?}; adapted-lens scans have no usable profile: borrowed Sony warps below are diagnostic resampling only, not calibrated lens correction",
        train.fixed_wb
    );
    for (i, profile) in profiles.iter().enumerate() {
        let geometry = EmbeddedLensGeometry::new(profile.clone(), train.area).unwrap();
        for tile in [false, true] {
            let label = format!(
                "film-train-borrowed{}-{}",
                if i == 0 { 24 } else { 70 },
                if tile { "detail" } else { "global" }
            );
            let mut data = sample(&train, Some(&geometry), tile, &label, Some(calibration));
            report(&data, &gpu, &output_dir, &mut csv);
            if i == 0 {
                data.flat = true;
                data.label.push_str("-flat");
                report(&data, &gpu, &output_dir, &mut csv);
                data.spatial = true;
                data.label.push_str("-spatial");
                report(&data, &gpu, &output_dir, &mut csv);
            }
        }
    }
    let wb = train.fixed_wb;
    drop(train);
    let dense = Scan::open(&root.join("DSC04378.ARW"), Some(wb)).unwrap();
    let geometry = EmbeddedLensGeometry::new(profiles[0].clone(), dense.area).unwrap();
    for tile in [false, true] {
        let mut data = sample(
            &dense,
            Some(&geometry),
            tile,
            if tile {
                "film-dmax-borrowed24-detail"
            } else {
                "film-dmax-borrowed24-global"
            },
            Some(calibration),
        );
        report(&data, &gpu, &output_dir, &mut csv);
        if tile {
            data.flat = true;
            data.spatial = true;
            data.label.push_str("-flat-spatial");
            report(&data, &gpu, &output_dir, &mut csv);
        }
    }
    drop(dense);
    for name in ["DSC00638", "DSC00632", "DSC00666", "DSC00635"] {
        let scan = Scan::open(&root.join("non-scans").join(format!("{name}.ARW")), None).unwrap();
        report(
            &sample(&scan, None, false, name, None),
            &gpu,
            &output_dir,
            &mut csv,
        );
    }
    println!("precision results={}", output_dir.display());
}
