#![cfg(all(feature = "gpu", target_os = "macos"))]
use antitone::adjustment_layers::AdjustmentLayerSettings;

use antitone::{
    gpu::{ProcessingKernel, ProcessingParameters, ProcessingSource},
    processing::{CAMERA_INPUT_FLOOR, Calibration, RollSettings},
};
use glam::{DMat3, DVec3};

#[path = "support/gpu_headroom.rs"]
mod headroom;

async fn context() -> (wgpu::Device, wgpu::Queue, ProcessingKernel) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = instance.request_adapter(&Default::default()).await.unwrap();
    let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
    let kernel = ProcessingKernel::new(&device).await.unwrap();
    (device, queue, kernel)
}

fn process(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    kernel: &ProcessingKernel,
    source: &ProcessingSource,
    count: usize,
    parameters: ProcessingParameters,
    grade: Option<&antitone::gpu::GpuAdjustmentLayers>,
) -> Vec<[f32; 4]> {
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("continuous camera test"),
        size: count as u64 * 16,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: output.size(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    if let Some(grade) = grade {
        kernel
            .encode_grade(device, &mut encoder, source, &output, grade)
            .unwrap();
    } else {
        kernel.encode(device, &mut encoder, source, &output, parameters);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, output.size());
    queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |r| send.send(r).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    receive.recv().unwrap().unwrap();
    let mapped = readback.slice(..).get_mapped_range().unwrap();
    let result = bytemuck::cast_slice(&mapped).to_vec();
    drop(mapped);
    readback.unmap();
    result
}

#[test]
fn fractional_camera_values_survive_source_inspection_and_continuous_inversion() {
    pollster::block_on(async {
        let (device, queue, kernel) = context().await;
        // A non-workgroup-sized raster catches source stride/count errors in the float path.
        let pixels: Vec<_> = (0..67)
            .map(|i| {
                [
                    1000.25 + i as f32 * 467.5,
                    2000.75 + i as f32 * 123.25,
                    5000.125 + i as f32 * 91.125,
                    0.,
                ]
            })
            .collect();
        let source = ProcessingSource::new_continuous(&device, &pixels, DMat3::IDENTITY).unwrap();
        assert_eq!(source.byte_len(), pixels.len() as u64 * 16);
        let inspected = process(
            &device,
            &queue,
            &kernel,
            &source,
            pixels.len(),
            ProcessingParameters::source(),
            None,
        );
        for (pixel, actual) in pixels.iter().zip(inspected) {
            for c in 0..3 {
                assert_eq!(actual[c], (f64::from(pixel[c]) / 65535.) as f32);
            }
            assert_eq!(actual[3], 0.);
        }
        let calibration = Calibration {
            dmin: DVec3::splat(0.7),
            dmax: DVec3::splat(0.01),
            input_floor: CAMERA_INPUT_FLOOR,
        };
        let roll = RollSettings::default();
        let photo = AdjustmentLayerSettings {
            exposure: 4.,
            ..Default::default()
        };
        let parameters =
            ProcessingParameters::positive(calibration, roll, DMat3::IDENTITY).unwrap();
        let grade = antitone::gpu::GpuAdjustmentLayers::new(
            &device,
            calibration,
            roll,
            antitone::adjustment_layers::AdjustmentLayerSnapshot::new(vec![
                antitone::adjustment_layers::AdjustmentLayer::global(photo),
            ])
            .unwrap(),
            DMat3::IDENTITY,
            [0, 0, pixels.len() as u32, 1].into(),
        )
        .unwrap();
        let actual = process(
            &device,
            &queue,
            &kernel,
            &source,
            pixels.len(),
            parameters,
            Some(&grade),
        );
        let reference = antitone::adjustment_layers::AdjustmentLayerSnapshot::new(vec![
            antitone::adjustment_layers::AdjustmentLayer::global(photo),
        ])
        .and_then(|layers| {
            antitone::adjustment_layers::AdjustmentLayerPipeline::new(
                calibration,
                roll,
                layers,
                DMat3::IDENTITY,
            )
        })
        .unwrap();
        for (pixel, actual) in pixels.iter().zip(actual) {
            let expected = reference
                .apply(
                    DVec3::new(pixel[0] as f64, pixel[1] as f64, pixel[2] as f64) / 65535.,
                    [0, 0],
                )
                .unwrap();
            for (c, &value) in actual[..3].iter().enumerate() {
                assert!(
                    (value as f64 - expected.rgb[c]).abs() <= 2e-5 + 2e-4 * expected.rgb[c].abs()
                );
            }
            assert_eq!(actual[3], 0.);
        }
        assert!(source.write(&queue, &[[0; 4]; 67]).is_err());
        assert!(
            ProcessingSource::new_continuous(&device, &[[f32::NAN, 0., 0., 0.]], DMat3::IDENTITY)
                .is_err()
        );
        assert!(
            ProcessingSource::new_continuous(&device, &[[-0.1, 0., 0., 0.]], DMat3::IDENTITY)
                .is_err()
        );
    });
}

#[test]
fn camera_gain_headroom_is_checked_before_a_compensating_matrix() {
    use antitone::{
        flat_field::FlatField,
        gpu::{GpuFlatField, SourceGeometry},
    };
    use std::sync::Arc;

    pollster::block_on(async {
        let (device, queue, kernel) = context().await;
        let matrix = DMat3::from_diagonal(DVec3::new(2f64.powi(-80), 2f64.powi(40), 2f64.powi(40)));
        let pixels = [[8. * 65535., 65535., 65535., 0.]];
        let mut source = ProcessingSource::new_continuous(&device, &pixels, matrix).unwrap();

        for exponent in [126, 110] {
            let gain = 2f64.powi(exponent);
            let field = Arc::new(
                GpuFlatField::new(
                    &device,
                    Arc::new(
                        FlatField::from_nodes(
                            [0, 0, 1, 1],
                            [2, 2],
                            vec![DVec3::new(gain, 1., 1.); 4],
                        )
                        .unwrap(),
                    ),
                )
                .unwrap(),
            );
            let result = source.set_flat_field(
                Some(field),
                SourceGeometry::new([0, 0, 1, 1], [1, 1]).unwrap(),
            );
            if exponent == 126 {
                // The final red value fits, but applying the gain before the matrix overflows.
                assert!(result.is_err());
            } else {
                result.unwrap();
                let observed = process(
                    &device,
                    &queue,
                    &kernel,
                    &source,
                    1,
                    ProcessingParameters::source(),
                    None,
                );
                let expected = matrix * DVec3::new(8. * gain, 1., 1.);
                for c in 0..3 {
                    assert!(observed[0][c].is_finite());
                    assert_eq!(observed[0][c] as f64, expected[c]);
                }
            }
        }
    });
}

#[test]
fn integer_values_preserve_output_bits_in_native_and_continuous_transport() {
    pollster::block_on(async {
        let (device, queue, kernel) = context().await;
        let codes: Vec<[u16; 4]> = (0..67u32)
            .map(|i| [(i * 977) as u16, (i * 431) as u16, (i * 167) as u16, 0])
            .collect();
        let floats: Vec<_> = codes.iter().map(|p| p.map(f32::from)).collect();
        let integer = ProcessingSource::new(&device, &codes, DMat3::IDENTITY).unwrap();
        let continuous =
            ProcessingSource::new_continuous(&device, &floats, DMat3::IDENTITY).unwrap();
        let calibration = Calibration {
            dmin: DVec3::splat(0.7),
            dmax: DVec3::splat(0.01),
            input_floor: CAMERA_INPUT_FLOOR,
        };
        let parameters =
            ProcessingParameters::positive(calibration, RollSettings::default(), DMat3::IDENTITY)
                .unwrap();
        assert_eq!(
            process(
                &device,
                &queue,
                &kernel,
                &integer,
                codes.len(),
                parameters,
                None
            ),
            process(
                &device,
                &queue,
                &kernel,
                &continuous,
                codes.len(),
                parameters,
                None
            )
        );
    });
}
