#![cfg(all(feature = "raw", feature = "gpu", target_os = "macos"))]
use antitone as api;
use api::{
    color::Balance,
    gpu::{ProcessingKernel, ProcessingParameters, ProcessingSource},
    processing::{PhotoSettings, Pipeline, RollSettings},
};
use glam::Vec4;
#[path = "support/train_raw.rs"]
mod train_raw;

#[test]
#[ignore = "read-only local train RAW and diagnostic calibration; run explicitly in release"]
fn train_native_source_and_gpu_match_scalar_with_unchanged_numerical_gate() {
    let (train, cal) = train_raw::load();
    let (_, _, codes) = train.preview_source(512).unwrap();
    let roll = RollSettings {
        exposure: 0.5,
        balance: Balance {
            warmth: 0.17,
            tint: -0.11,
        },
        ..RollSettings::new_grade()
    };
    let photo = PhotoSettings {
        exposure: 0.25,
        wb: Balance {
            warmth: -0.15,
            tint: 0.12,
        },
        ..PhotoSettings::default()
    };
    let pipeline = Pipeline::new(cal, roll, photo, train.camera_to_ap1).unwrap();
    let params = ProcessingParameters::positive(cal, roll, photo, train.camera_to_ap1).unwrap();
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let source =
            ProcessingSource::new_continuous(&device, &codes, train.camera_to_ap1).unwrap();
        assert_eq!(source.byte_len(), codes.len() as u64 * 16);
        let kernel = ProcessingKernel::new(&device).await.unwrap();
        let bytes = codes.len() as u64 * 16;
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        kernel.encode(&device, &mut encoder, &source, &output, params);
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
        queue.submit([encoder.finish()]);
        let (send, receive) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |r| send.send(r).unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        receive.recv().unwrap().unwrap();
        let mapped = readback.slice(..).get_mapped_range().unwrap();
        let actual: &[Vec4] = bytemuck::cast_slice(&mapped);
        let mut worst = 0_f64;
        for (code, actual) in codes.iter().zip(actual) {
            let camera = glam::DVec3::new(code[0] as f64, code[1] as f64, code[2] as f64) / 65535.;
            let reference = pipeline.apply(camera).unwrap();
            let flags = (0..3).fold(0u32, |bits, c| {
                bits | (u32::from(reference.floored[c]) << c)
            });
            assert_eq!(actual.w as u32, flags);
            for c in 0..3 {
                let ratio = (actual[c] as f64 - reference.rgb[c]).abs()
                    / (2e-5 + 2e-4 * reference.rgb[c].abs());
                assert!(
                    actual[c].is_finite() && ratio <= 1.,
                    "camera={camera:?} actual={actual:?} reference={reference:?}"
                );
                worst = worst.max(ratio);
            }
        }
        println!(
            "train model-4 native RGB16 samples={} worst tolerance multiple={worst:.9e}; diagnostic anchors, not photographic calibration",
            codes.len()
        );
    });
}
