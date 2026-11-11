#![cfg(feature = "raw")]

use antitone::scan::Scan;
use std::f64::consts::TAU;

#[path = "support/sony_fixture.rs"]
mod fixture;
#[path = "support/fixture_path.rs"]
mod fixture_path;
#[cfg(all(feature = "gpu", feature = "export", feature = "project"))]
#[path = "support/output.rs"]
mod output;

const BASE: [f64; 3] = [837., 537., 133.];
const BLACK: f64 = 512.;
const RANGE: f64 = 16383. - BLACK;

fn channel(x: u32, y: u32) -> usize {
    match (x % 2, y % 2) {
        (0, 0) => 0,
        (1, 1) => 2,
        _ => 1,
    }
}

fn source(rgb: impl Fn(u32, u32) -> [f64; 3]) -> fixture_path::FixturePath {
    fixture::source_with_pixels(|x, y| {
        let c = channel(x, y);
        let light = if y < 4 && x < 8 {
            // Distant extrema prevent a flat local channel from passing merely
            // because reconstruction constrains it to the global channel range.
            BASE[c] * if x < 4 { 0.1 } else { 4. }
        } else {
            rgb(x, y)[c]
        };
        (BLACK + light.round()) as u16
    })
}

fn open(file: &fixture_path::FixturePath) -> Scan {
    let scan = Scan::open(file.path(), Some([1.; 4])).unwrap();
    assert_eq!(scan.dimensions().unwrap(), (64, 64));
    assert_eq!(scan.white_balance, [1.; 4]);
    assert_eq!(scan.source().metadata().black, BLACK as u32);
    assert_eq!(scan.source().metadata().maximum, 16383);
    scan
}

fn noise(x: u32, y: u32) -> f64 {
    let h = (y * 64 + x)
        .wrapping_mul(747796405)
        .wrapping_add(2891336453);
    let h = ((h >> ((h >> 28) + 4)) ^ h).wrapping_mul(277803737);
    f64::from(((h >> 22) ^ h) % 65) - 32.
}

#[test]
fn weak_blue_reconstruction_does_not_amplify_green_only_noise_into_strong_colour_texture() {
    let rgb = |x, y| [BASE[0], BASE[1] + noise(x, y), BASE[2]];
    let file = source(rgb);
    let scan = open(&file);
    let mut blue_error = 0.;
    let mut green_noise = 0.;
    let mut green_sites = 0;
    for y in 16..48 {
        for x in 16..48 {
            let pixel = scan.pixel(x, y).unwrap();
            let c = channel(x, y);
            let expected = rgb(x, y)[c].round() / RANGE;
            assert!(
                (pixel[c] - expected).abs() <= 1. / 65535. + 2e-8,
                "measured channel changed at {x},{y}: {pixel:?}"
            );
            blue_error += (pixel.z / (BASE[2] / RANGE) - 1.).powi(2);
            if c == 1 {
                green_noise += (noise(x, y) / BASE[1]).powi(2);
                green_sites += 1;
            }
        }
    }
    let blue_rms = (blue_error / 1024.).sqrt();
    let green_rms = (green_noise / f64::from(green_sites)).sqrt();
    assert!(green_rms > 0.03, "the authored noise must be active");
    // A reconstruction allowance, not a sensor-noise estimate: borrowing green
    // texture must not magnify its fractional amplitude in the weaker channel.
    assert!(blue_rms <= 1.5 * green_rms, "{blue_rms} vs {green_rms}");
}

fn amplitude(scan: &Scan, frequency: f64, c: usize) -> f64 {
    let mut sin = 0.;
    let mut cos = 0.;
    for y in 16..48 {
        for x in 16..48 {
            let phase = TAU * frequency * f64::from(x);
            let value = scan.pixel(x, y).unwrap()[c] / (BASE[c] / RANGE);
            sin += value * phase.sin();
            cos += value * phase.cos();
        }
    }
    // Both fixtures contain an integral number of periods in this region.
    // The authored modulation is 0.25; the DC term is orthogonal to sin/cos.
    (sin.hypot(cos) * 2. / 1024.) / 0.25
}

#[test]
fn unbalanced_bayer_luminance_detail_retains_modulation_without_blue_overamplification() {
    let frequency = 3. / 16.;
    let file = source(|x, _| {
        let value = 1. + 0.25 * (TAU * frequency * f64::from(x)).sin();
        BASE.map(|v| v * value)
    });
    let scan = open(&file);
    for c in 0..3 {
        let measured = amplitude(&scan, frequency, c);
        // Below the red/blue CFA Nyquist limit: a 15% contrast allowance catches
        // blanket bilinear smoothing as well as transferred-green blue ringing.
        assert!((0.85..=1.15).contains(&measured), "{c}: {measured}");
    }
}

#[test]
fn independently_varying_blue_detail_is_not_suppressed_with_borrowed_green_texture() {
    let frequency = 1. / 16.;
    let file = source(|x, _| {
        let mut rgb = BASE;
        rgb[2] *= 1. + 0.25 * (TAU * frequency * f64::from(x)).sin();
        rgb
    });
    let scan = open(&file);
    let measured = amplitude(&scan, frequency, 2);
    assert!((0.9..=1.05).contains(&measured), "{measured}");
    let mut sin = 0.;
    let mut cos = 0.;
    let mut leakage = [0.; 2];
    for y in 16..48 {
        for x in 16..48 {
            let pixel = scan.pixel(x, y).unwrap();
            let phase = TAU * frequency * f64::from(x);
            let blue = pixel.z / (BASE[2] / RANGE);
            sin += blue * phase.sin();
            cos += blue * phase.cos();
            for c in 0..2 {
                leakage[c] += (pixel[c] / (BASE[c] / RANGE) - 1.).powi(2);
            }
        }
    }
    // A quarter-original-pixel registration allowance distinguishes retained
    // chromatic detail from a shifted signal with the same modulation.
    assert!(cos.atan2(sin).abs() <= TAU * frequency * 0.25);
    for squared in leakage {
        assert!((squared / 1024.).sqrt() <= 0.02);
    }
}

#[test]
fn slanted_chromatic_edge_retains_its_independent_red_and_blue_changes() {
    let rgb = |x, y| {
        let distance = f64::from(x) - 32. - 0.2 * (f64::from(y) - 32.);
        let edge = 0.5 + 0.5 * (distance / 0.75).tanh();
        [
            BASE[0] * (0.75 + 0.5 * edge),
            BASE[1],
            BASE[2] * (1.25 - 0.5 * edge),
        ]
    };
    let file = source(rgb);
    let scan = open(&file);
    let mut error = [0.; 3];
    for y in 16..48 {
        for x in 16..48 {
            let actual = scan.pixel(x, y).unwrap();
            for c in 0..3 {
                error[c] += ((actual[c] * RANGE - rgb(x, y)[c]) / BASE[c]).powi(2);
            }
        }
    }
    for (c, squared) in error.into_iter().enumerate() {
        let rms = (squared / 1024.).sqrt();
        assert!(rms < 0.05, "chromatic step reconstruction {c}: {rms}");
    }
    let left = scan.pixel(16, 32).unwrap();
    let right = scan.pixel(47, 32).unwrap();
    assert!(right.x > left.x * 1.5 && right.z < left.z / 1.5);
}

#[cfg(all(feature = "gpu", feature = "export", feature = "project"))]
#[test]
fn reconstructed_negative_detail_reaches_original_sample_gpu_export_in_the_same_camera_basis() {
    use antitone::{
        color::ap1_to_ap0,
        crop::Crop,
        export::{ExrPrecision, Overwrite},
        geometry::{Composition, SourceRegion},
        gpu::submission::SubmissionGate,
        processing::{InputSettings, Response},
        renderer::GpuRenderer,
    };
    use glam::DVec3;
    use output::{self as gpu_export, Format, Request};

    let frequency = 3. / 16.;
    let file = source(|x, _| {
        let light = 1. + 0.25 * (TAU * frequency * f64::from(x)).sin();
        BASE.map(|v| v * light)
    });
    let scan = open(&file);
    let (device, queue, renderer) = pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let renderer = GpuRenderer::new(&device, &queue).await.unwrap();
        (device, queue, renderer)
    });
    let base = DVec3::from_array(BASE) / RANGE;
    let input = InputSettings::Negative {
        dmin: Some(base * 2.),
        dmax: Some(base * 0.5),
        response: Response {
            toe: 1.,
            shoulder: 0.,
        },
        gamma: 1.,
        balance: Default::default(),
    };
    let composition = Composition::with_crop(
        scan.area,
        Default::default(),
        Default::default(),
        Some(Crop::from_region(SourceRegion {
            x: 16,
            y: 16,
            width: 32,
            height: 32,
        })),
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("detail.exr");
    gpu_export::write(
        &renderer,
        &device,
        (&queue, &SubmissionGate::default()),
        Request {
            scan: &scan,
            field: None,
            input,
            roll: Default::default(),
            grade: antitone::adjustment_layers::AdjustmentLayerSnapshot::new(vec![]).unwrap(),
            repairs: &[],
            composition,
            format: Format::Exr(ExrPrecision::Full),
            luts: Default::default(),
            pipeline: Default::default(),
        },
        &path,
        Overwrite::Refuse,
    )
    .unwrap();
    let image = exr::prelude::read_first_flat_layer_from_file(path).unwrap();
    let matrix = ap1_to_ap0() * scan.camera_to_ap1;
    assert_eq!(image.layer_data.channel_data.list.len(), 3);
    for channel in &image.layer_data.channel_data.list {
        let c = match channel.name.to_string().as_str() {
            "R" => 0,
            "G" => 1,
            "B" => 2,
            _ => panic!("unexpected channel"),
        };
        let exr::prelude::FlatSamples::F32(values) = &channel.sample_data else {
            panic!("expected full precision scene output")
        };
        assert_eq!(values.len(), 32 * 32);
        let squared: f64 = values
            .iter()
            .enumerate()
            .map(|(i, value)| {
                let x = 16 + (i % 32) as u32;
                let light = 1. + 0.25 * (TAU * frequency * f64::from(x)).sin();
                // These supplied anchors and a gamma/toe of one give this simple
                // analytic positive, independent of the production inversion code.
                let positive = (2. / light - 1.) / 3.;
                let expected = (matrix * DVec3::splat(positive))[c];
                (f64::from(*value) - expected).powi(2)
            })
            .sum();
        let rms = (squared / values.len() as f64).sqrt();
        // A 2% positive-light reconstruction budget propagated through the
        // declared matrix; this catches erased detail or the old blue gain.
        let allowance = 0.02 * matrix.row(c).abs().element_sum();
        assert!(rms <= allowance, "{c}: {rms} > {allowance}");
    }
}
