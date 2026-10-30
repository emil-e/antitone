use super::*;
use crate::gpu::camera_fixture::CameraFixture;

mod adjustment_layer_performance;
mod camera_raw;
mod gpu_adjustment_layers;
mod gpu_continuous;
mod gpu_film_response;
mod gpu_processing;
mod lens_precision;
use crate::adjustment_layers::{AdjustmentLayer, AdjustmentLayerSettings, AdjustmentLayerSnapshot};
use crate::processing::Calibration;

#[test]
fn processing_assembles_shared_photographic_operators_once() {
    let shader = CameraResources::kernel_shader();
    assert!(!shader.contains("Triple"));
    assert!(!shader.contains("precise_"));
    // Originals, repairs and working views use the same photographic operators.
    for name in [
        "invert_film",
        "colour_encode",
        "colour_ratio",
        "photographic_post",
    ] {
        assert_eq!(shader.matches(&format!("fn {name}(")).count(), 1, "{name}");
    }
}

#[test]
fn source_inspection_is_zero_default_and_changes_only_the_source_post_matrix() {
    let base = ProcessingParameters::source();
    assert_eq!(
        bytemuck::bytes_of(&base),
        bytemuck::bytes_of(&ProcessingParameters::source_inspection(0.).unwrap())
    );
    for exposure in [-16.0_f64, -2., 2., 16.] {
        let gain = exposure.exp2();
        let adjusted = ProcessingParameters::source_inspection(exposure).unwrap();
        assert_eq!(adjusted.post, matrix_columns(DMat3::IDENTITY * gain));
        let unchanged = ProcessingParameters {
            post: base.post,
            ..adjusted
        };
        assert_eq!(bytemuck::bytes_of(&unchanged), bytemuck::bytes_of(&base));
    }
    for exposure in [f64::NAN, f64::INFINITY, -17., 17.] {
        assert!(ProcessingParameters::source_inspection(exposure).is_err());
    }
}

#[test]
fn layer_output_bounds_do_not_propagate_scratch_peaks_into_following_layers() {
    let calibration = Calibration {
        dmin: DVec3::new(0.19, 0.14, 0.12),
        dmax: DVec3::new(0.008, 0.004, 0.002),
        input_floor: 1. / 65535.,
    };
    let (_, input) = ProcessingParameters::positive_with_bound(
        calibration,
        RollSettings::default(),
        DMat3::IDENTITY,
    )
    .unwrap();
    for settings in [
        AdjustmentLayerSettings {
            contrast: 1.05,
            ..Default::default()
        },
        AdjustmentLayerSettings {
            saturation: 2.,
            ..Default::default()
        },
        AdjustmentLayerSettings {
            contrast: 1.05,
            saturation: 1.2,
            ..Default::default()
        },
    ] {
        let grade = crate::photographic::Grade::new(settings).unwrap();
        let (peak, output) = grade.weighted_bounds(input, 1.);
        assert!(output < peak, "{settings:?}: {output} >= {peak}");
    }
    // Saturation 2 has output bound 3B and scratch bound 5B. Two layers fit
    // at B=M/20; propagating scratch instead of output would require 25B > M.
    let settings = AdjustmentLayerSettings {
        saturation: 2.,
        ..Default::default()
    };
    let matrix = DMat3::from_diagonal(DVec3::splat(f64::from(f32::MAX) / (80. * input)));
    let snapshot = AdjustmentLayerSnapshot::new(vec![
        AdjustmentLayer::global(settings),
        AdjustmentLayer::global(settings),
    ])
    .unwrap();
    assert!(
        ProcessingInput::validate_limits(
            &wgpu::Limits::default(),
            calibration,
            RollSettings::default(),
            &snapshot,
            matrix,
            [0, 0, 1, 1].into()
        )
        .is_ok()
    );
    let matrix = DMat3::from_diagonal(DVec3::splat(f64::from(f32::MAX) / (16. * input)));
    let settings = AdjustmentLayerSettings {
        contrast: 0.4,
        ..Default::default()
    };
    let snapshot = AdjustmentLayerSnapshot::new(vec![AdjustmentLayer::global(settings)]).unwrap();
    assert!(
        ProcessingInput::validate_limits(
            &wgpu::Limits::default(),
            calibration,
            RollSettings::default(),
            &snapshot,
            matrix,
            crate::geometry::SourceRegion {
                x: 0,
                y: 0,
                width: 1,
                height: 1
            }
        )
        .is_err(),
        "an overflowing layer scratch intermediate must reject the complete stack"
    );
}

#[test]
fn source_preview_restores_green_referenced_headroom_and_keeps_inspection_relative() {
    for (wb, headroom) in [
        ([1.; 4], 1.),
        ([2., 1., 1.5, 1.], 2.),
        ([0.25, 1., 0.5, 1.], 1.),
        ([1., 2., 4., 2.], 2.),
        ([1., 1., 0.5, 3.5], 3.5),
        ([1024., 512., 768., 512.], 2.),
    ] {
        for exposure in [-16.0_f64, -2., 0., 2., 16.] {
            let adjusted = ProcessingParameters::source_preview(Some(wb), exposure).unwrap();
            assert_eq!(
                adjusted.post,
                matrix_columns(DMat3::IDENTITY * (2. * headroom * exposure.exp2()))
            );
            let base = ProcessingParameters::source();
            let unchanged = ProcessingParameters {
                post: base.post,
                ..adjusted
            };
            assert_eq!(bytemuck::bytes_of(&unchanged), bytemuck::bytes_of(&base));
        }
    }
    for exposure in [f64::NAN, f64::INFINITY, -17., 17.] {
        assert!(ProcessingParameters::source_preview(Some([1.; 4]), exposure).is_err());
    }
    // Preserve the low-level identity constructor for processing diagnostics.
    assert_eq!(
        ProcessingParameters::source().post,
        matrix_columns(DMat3::IDENTITY)
    );
}

#[test]
fn source_preview_falls_back_without_repairing_missing_or_unusable_wb() {
    for wb in [
        None,
        Some([0.; 4]),
        Some([1., 1., 1., 0.]),
        Some([1., 0., 1., 1.]),
        Some([1., f64::NAN, 1., 1.]),
        Some([f64::INFINITY, 1., 1., 1.]),
        Some([-1., 1., 1., 1.]),
        Some([f64::MAX, 1., 1., 1.]),
        Some([f64::MIN_POSITIVE; 4]),
        Some([1e-6; 4]),
        Some([1., 1e-30, 1., 1.]),
        Some([65537., 1., 1., 1.]),
    ] {
        for exposure in [-16.0_f64, 0., 16.] {
            let parameters = ProcessingParameters::source_preview(wb, exposure).unwrap();
            assert_eq!(
                parameters.post,
                matrix_columns(DMat3::IDENTITY * (2. * exposure.exp2()))
            );
        }
    }
    assert_eq!(source_preview_gain(Some([65536., 1., 1., 1.])), 131072.);
}

#[cfg(feature = "raw")]
#[test]
fn authored_source_preview_scale_agrees_with_actual_libraw_development_gains() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng");
    for wb in [
        [1.; 4],
        [2.13, 1., 1.77, 1.],
        [0.5, 1., 0.25, 0.75],
        [1., 1., 0.5, 3.5],
    ] {
        let mut raw = crate::raw::RawFile::open_uncorrected(&path).unwrap();
        let image = raw.develop_camera(wb).unwrap();
        let before = image.pixel_codes(32, 32).unwrap();
        let gain = 2. / image.pre_mul[1];
        let parameters = ProcessingParameters::source_preview(Some(wb), 0.).unwrap();
        assert!((f64::from(parameters.post[0].x) - gain).abs() < 1e-6 * gain);
        assert_eq!(image.pixel_codes(32, 32).unwrap(), before);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn source_inspection_gpu_matches_scaled_camera_to_ap1_with_zero_diagnostic_flags() {
    pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let codes = [
            [16384, 8192, 4096, 0],
            [0, 0, 0, 5432],
            [65535, 65535, 65535, 0],
            [4000, 16000, 8000, 65535],
        ];
        let matrix = DMat3::from_cols(
            DVec3::new(0.6, 0.02, -0.01),
            DVec3::new(0.4, 1.2, -0.15),
            DVec3::new(0., -0.22, 1.16),
        );
        let source = ProcessingSource::new(&device, &codes, matrix).unwrap();
        let kernel = CameraFixture::new(&device).await.unwrap();
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cases = [-16.0_f64, -2., 0., 2., 16.]
            .into_iter()
            .flat_map(|exposure| {
                [
                    (
                        ProcessingParameters::source_inspection(exposure).unwrap(),
                        exposure.exp2(),
                    ),
                    (
                        ProcessingParameters::source_preview(Some([2., 1., 1.5, 1.]), exposure)
                            .unwrap(),
                        4. * exposure.exp2(),
                    ),
                    (
                        ProcessingParameters::source_preview(Some([0.5, 1., 0.25, 1.]), exposure)
                            .unwrap(),
                        2. * exposure.exp2(),
                    ),
                ]
            });
        let transform = DMat3::from_cols(
            DVec3::new(1.4, 0.1, -0.05),
            DVec3::new(-0.2, 1.1, 0.05),
            DVec3::new(0.1, -0.1, 0.8),
        );
        let cases = cases
            .map(|(parameters, gain)| (parameters, DMat3::IDENTITY * gain))
            .chain([(
                ProcessingParameters::source_preview_with_transform(Some([1.; 4]), 0.37, transform)
                    .unwrap(),
                transform * (2. * 0.37_f64.exp2()),
            )]);
        for (parameters, post) in cases {
            let mut encoder = device.create_command_encoder(&Default::default());
            kernel.encode(&device, &mut encoder, &source, &output, parameters);
            encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 64);
            queue.submit([encoder.finish()]);
            let (send, receive) = std::sync::mpsc::channel();
            readback
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    send.send(result).unwrap()
                });
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            receive.recv().unwrap().unwrap();
            let mapped = readback.slice(..).get_mapped_range().unwrap();
            let values: &[Vec4] = bytemuck::cast_slice(&mapped);
            for (pixel, actual) in codes.iter().zip(values) {
                let camera = DVec3::new(pixel[0] as f64, pixel[1] as f64, pixel[2] as f64) / 65535.;
                let expected = post * matrix * camera;
                for channel in 0..3 {
                    assert!(
                        (actual[channel] as f64 - expected[channel]).abs()
                            <= 2e-5 + 2e-4 * expected[channel].abs()
                    );
                }
                assert_eq!(actual.w, 0.);
            }
            drop(mapped);
            readback.unmap();
        }
    });
}

#[test]
fn camera_matrix_rejects_nonfinite_and_overflowing_values() {
    for value in [f64::NAN, f64::INFINITY, 1e300] {
        assert!(CameraMatrix::new(DMat3::from_diagonal(DVec3::splat(value))).is_err());
    }
    assert!(CameraMatrix::new(DMat3::IDENTITY).is_ok());
}

#[cfg(target_os = "macos")]
#[test]
fn native_and_continuous_transport_preserve_normalization_and_source_addressing() {
    pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let kernel = CameraFixture::new(&device).await.unwrap();
        let codes: Vec<_> = (0..=65535u16).map(|v| [v, 65535 - v, v / 2, 0]).collect();
        let continuous: Vec<_> = codes.iter().map(|p| p.map(f32::from)).collect();
        let native = ProcessingSource::new(&device, &codes, DMat3::IDENTITY).unwrap();
        let float =
            ProcessingSource::new_continuous(&device, &continuous, DMat3::IDENTITY).unwrap();
        assert_eq!(native.byte_len(), codes.len() as u64 * 8);
        assert!(native.write(&queue, &codes[..1]).is_err());
        assert!(ProcessingSource::new(&device, &[], DMat3::IDENTITY).is_err());
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: codes.len() as u64 * 16,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: output.size(),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut previous = None;
        for source in [&native, &float] {
            let mut encoder = device.create_command_encoder(&Default::default());
            kernel.encode(
                &device,
                &mut encoder,
                source,
                &output,
                ProcessingParameters::source(),
            );
            encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, output.size());
            queue.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            readback.map_async(wgpu::MapMode::Read, .., move |r| tx.send(r).unwrap());
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            rx.recv().unwrap().unwrap();
            let mapped = readback.get_mapped_range(..).unwrap();
            let data: &[[f32; 4]] = bytemuck::cast_slice(&mapped);
            for (pixel, code) in data.iter().zip(&codes) {
                for c in 0..3 {
                    // Half a native code distinguishes source/normalization mistakes from f32 rounding.
                    assert!((pixel[c] * 65535. - f32::from(code[c])).abs() < 0.5);
                }
                assert_eq!(pixel[3], 0.);
            }
            if let Some(previous) = &previous {
                assert_eq!(
                    data, previous,
                    "integer samples must not depend on transport"
                );
            } else {
                previous = Some(data.to_vec());
            }
            drop(mapped);
            readback.unmap();
        }
    });
}

#[test]
fn digital_repair_bounds_apply_roll_exposure_once() {
    let neutral = DVec3::splat(1e30);
    for exposure in [-16., 0., 16.] {
        let (_, bound) = ProcessingParameters::digital_with_neutral_bound(
            RollSettings {
                exposure,
                ..Default::default()
            },
            DMat3::IDENTITY,
            neutral,
        )
        .unwrap();
        assert_eq!(bound, neutral.x * exposure.exp2());
    }
}
