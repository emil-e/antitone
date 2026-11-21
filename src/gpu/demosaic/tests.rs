use super::{Demosaicer, Layout};

const PATTERNS: [[u8; 4]; 4] = [[0, 1, 1, 2], [2, 1, 1, 0], [1, 0, 2, 1], [1, 2, 0, 1]];

fn gpu() -> Demosaicer {
    static DEVICE: std::sync::OnceLock<(wgpu::Device, crate::gpu::submission::Queue)> =
        std::sync::OnceLock::new();
    let (device, queue) = DEVICE.get_or_init(|| {
        pollster::block_on(async {
            let adapter = crate::gpu::instance()
                .request_adapter(&Default::default())
                .await
                .unwrap();
            let supported = adapter.limits();
            crate::gpu::submission::request_device(
                &adapter,
                &wgpu::DeviceDescriptor {
                    required_limits: wgpu::Limits {
                        max_storage_buffers_per_shader_stage: 10,
                        max_storage_buffer_binding_size: supported.max_storage_buffer_binding_size,
                        max_buffer_size: supported.max_buffer_size,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .await
            .unwrap()
        })
    });
    Demosaicer::new(device, queue, 4 << 30)
}

fn image(w: u32, h: u32, pattern: [u8; 4], base: [u16; 3]) -> Vec<[u16; 4]> {
    (0..w * h)
        .map(|i| {
            let mut pixel = [55555; 4];
            let channel = pattern[(((i / w) % 2) * 2 + i % w % 2) as usize] as usize;
            pixel[channel] = base[channel];
            pixel
        })
        .collect()
}

#[test]
fn working_admission_counts_readback_and_upload_and_respects_device_limits() {
    let frame = Layout::new(100, 101, 10100, PATTERNS[0]).unwrap();
    assert_eq!(
        frame.working_bytes,
        frame.sizes.iter().sum::<u64>() + 2 * frame.image_bytes + 32
    );
    let mut limits = wgpu::Limits {
        max_storage_buffers_per_shader_stage: 10,
        ..Default::default()
    };
    assert!(frame.admit(&limits, frame.working_bytes).is_ok());
    assert!(frame.admit(&limits, frame.working_bytes - 1).is_err());
    limits.max_storage_buffer_binding_size = frame.image_bytes - 1;
    assert!(frame.admit(&limits, u64::MAX).is_err());
    limits = wgpu::Limits::default();
    assert!(frame.admit(&limits, u64::MAX).is_err());
    assert!(Layout::new(u32::MAX, u32::MAX, 1, PATTERNS[0]).is_err());
}

#[test]
fn analytic_fields_preserve_measurements_and_zero_light_across_phases_and_borders() {
    let gpu = gpu();
    for (w, h) in [(2, 2), (5, 9), (17, 14), (48, 32), (65, 63)] {
        for pattern in PATTERNS {
            for base in [
                [0; 3],
                [837, 537, 133],
                [65535; 3],
                [837, 537, 0],
                [0, 537, 133],
                [837, 0, 133],
            ] {
                let mut pixels = image(w, h, pattern, base);
                gpu.demosaic(&mut pixels, w, h, pattern).unwrap();
                for (i, pixel) in pixels.iter().enumerate() {
                    let i = i as u32;
                    let known = pattern[(((i / w) % 2) * 2 + i % w % 2) as usize] as usize;
                    assert_eq!(pixel[known], base[known]);
                    for c in 0..3 {
                        assert!(
                            (i32::from(pixel[c]) - i32::from(base[c])).abs() <= 1,
                            "{w}x{h} {pattern:?} {base:?} at {i}: {pixel:?}"
                        );
                        if base[c] == 0 {
                            assert_eq!(pixel[c], 0);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn rejection_leaves_input_unchanged_and_does_not_initialize_programs() {
    let mut gpu = gpu();
    let mut pixels = [[123u16; 4]; 4];
    assert!(gpu.demosaic(&mut pixels, 3, 2, PATTERNS[0]).is_err());
    assert!(gpu.demosaic(&mut pixels, 2, 2, [0, 0, 1, 2]).is_err());
    gpu.working_bytes = 0;
    assert!(gpu.demosaic(&mut pixels, 2, 2, PATTERNS[0]).is_err());
    assert_eq!(pixels, [[123; 4]; 4]);
    assert!(gpu.programs.get().is_none());
}

#[test]
fn memory_pressure_rejects_without_initializing_gpu_or_mutating_input() {
    let mut gpu = gpu();
    gpu.pressure = || crate::sys::MemoryPressure::Warning;
    let mut pixels = [[123u16; 4]; 4];
    assert!(gpu.demosaic(&mut pixels, 2, 2, PATTERNS[0]).is_err());
    assert_eq!(pixels, [[123; 4]; 4]);
    assert!(gpu.programs.get().is_none());
    gpu.pressure = || crate::sys::MemoryPressure::Normal;
    assert!(gpu.demosaic(&mut pixels, 2, 2, PATTERNS[0]).is_ok());
}

#[test]
fn adjacent_defects_do_not_spread_into_reconstructed_constant_channels() {
    let gpu = gpu();
    let (w, h) = (96, 96);
    let base = [837, 537, 133];
    for pattern in PATTERNS {
        for (a, b) in [(65535, 65000), (65000, 65535), (0, 1), (1, 0)] {
            let mut pixels = image(w, h, pattern, base);
            pixels[48 * w as usize + 48][pattern[0] as usize] = a;
            pixels[48 * w as usize + 49][pattern[1] as usize] = b;
            let source = pixels.clone();
            gpu.demosaic(&mut pixels, w, h, pattern).unwrap();
            for y in 40..60 {
                for x in 40..60 {
                    let i = y * w as usize + x;
                    let known = pattern[(y % 2) * 2 + x % 2] as usize;
                    assert_eq!(pixels[i][known], source[i][known]);
                    for (c, &expected) in base.iter().enumerate() {
                        if c != known {
                            assert!(
                                (i32::from(pixels[i][c]) - i32::from(expected)).abs() <= 1,
                                "{pattern:?} {a}/{b} {x},{y}: {:?}",
                                pixels[i]
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn failed_readback_after_submission_preserves_input_and_allows_retry() {
    let gpu = gpu();
    let mut image = image(17, 14, PATTERNS[0], [837, 537, 133]);
    let original = image.clone();
    let layout = Layout::new(17, 14, image.len(), PATTERNS[0]).unwrap();
    let programs = gpu.create_programs().unwrap();
    let mut frame = gpu.execute(&programs, &layout, &image).unwrap();
    // Force a real mapped-range error only after GPU execution and map completion.
    frame.readback.unmap();
    frame.mapping = false;
    assert!(frame.copy_into(&mut image).is_err());
    assert_eq!(image, original);
    drop(frame);
    gpu.demosaic(&mut image, 17, 14, PATTERNS[0]).unwrap();
    assert_ne!(
        image, original,
        "retry must reconstruct the missing channels"
    );
}

#[test]
fn simultaneous_callers_share_device_and_serialized_workspace_without_state_leakage() {
    let gpu = gpu();
    std::thread::scope(|scope| {
        let handles: Vec<_> = PATTERNS
            .into_iter()
            .enumerate()
            .map(|(phase, pattern)| {
                let gpu = &gpu;
                scope.spawn(move || {
                    let (w, h) = (47 + phase as u32, 35);
                    let mut a = image(w, h, pattern, [900, 500, 50]);
                    for (i, pixel) in a.iter_mut().enumerate() {
                        let known =
                            pattern[((i / w as usize) % 2) * 2 + i % w as usize % 2] as usize;
                        pixel[known] = if i % 17 == 0 {
                            65535
                        } else if i % 23 == 0 {
                            0
                        } else {
                            ((i * 7481 + 137) % 4096) as u16
                        };
                    }
                    let mut b = a.clone();
                    gpu.demosaic(&mut a, w, h, pattern).unwrap();
                    gpu.demosaic(&mut b, w, h, pattern).unwrap();
                    assert_eq!(a, b);
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
    });
}
