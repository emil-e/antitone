use super::*;

fn run(
    device: &wgpu::Device,
    queue: &crate::gpu::submission::Queue,
    kernel: &Rgb16,
    values: &[[f32; 4]],
) -> Vec<[u32; 2]> {
    let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(values),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: values.len() as u64 * 8,
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
    kernel
        .encode(device, &mut encoder, &input, &output, values.len() as u32)
        .unwrap();
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, output.size());
    let submission = queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    readback.map_async(wgpu::MapMode::Read, .., move |r| tx.send(r).unwrap());
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(10)),
        })
        .unwrap();
    rx.recv().unwrap().unwrap();
    let data = readback.get_mapped_range(..).unwrap();
    let words = bytemuck::cast_slice::<u8, [u32; 2]>(&data).to_vec();
    drop(data);
    readback.unmap();
    words
}

// IEC sRGB forward transfer expressed in unrounded RGB16 code units.
fn reference_code(linear: f32) -> f64 {
    let linear = f64::from(linear);
    let encoded = if linear <= 0.0031308 {
        12.92 * linear
    } else {
        1.055 * linear.powf(1. / 2.4) - 0.055
    };
    encoded * 65535.
}

#[test]
fn rgb16_recovers_code_centres_and_stays_within_one_code_of_the_srgb_transfer() {
    let (device, queue, _) = crate::photographic::test_support::render_context();
    const CENTRES: usize = 65536;
    const BLOCK: usize = 16384;

    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let kernel = Rgb16::new(&device, target).unwrap();
        let mut values: Vec<_> = (0..CENTRES)
            .map(|code| {
                let linear = target.decode(code as f64 / 65535.).unwrap() as f32;
                [linear, linear, linear, 0.]
            })
            .collect();
        for code in 0..65535 {
            let midpoint = target.decode((f64::from(code) + 0.5) / 65535.).unwrap() as f32;
            values.push([
                f32::from_bits(midpoint.to_bits() - 1),
                midpoint,
                f32::from_bits(midpoint.to_bits() + 1),
                0.,
            ]);
        }
        values.extend([
            [-0., 0., 1., 0.],
            [
                f32::from_bits(1),
                0.0031308,
                f32::from_bits(0.0031308_f32.to_bits() + 1),
                0.,
            ],
        ]);
        // Stratified linear values and deterministic float-bit samples supplement the boundaries.
        for i in 0..65536u32 {
            let bits = i.wrapping_mul(1664525).wrapping_add(1013904223) % (1_f32.to_bits() + 1);
            values.push([
                i as f32 / 65535.,
                f32::from_bits(bits),
                (65535 - i) as f32 / 65535.,
                0.,
            ]);
        }

        let mut worst = 0.0_f64;
        for (part, chunk) in values.chunks(BLOCK).enumerate() {
            let actual = decode(&run(&device, &queue, &kernel, chunk)).unwrap();
            assert_eq!(actual.len(), chunk.len());
            for (index, (&pixel, &input)) in actual.iter().zip(chunk).enumerate() {
                let index = part * BLOCK + index;
                if index < CENTRES {
                    assert_eq!(pixel, [index as u16; 3], "{target:?} code centre {index}");
                }
                for (code, linear) in pixel.into_iter().zip(input) {
                    let difference = (f64::from(code) - reference_code(linear)).abs();
                    // Includes the half-code quantization error; near a tie either adjacent
                    // integer is acceptable, but a transfer error of more than one code is not.
                    assert!(
                        difference <= 1.,
                        "{target:?}: {linear:?} → {code}, error {difference} codes"
                    );
                    worst = worst.max(difference);
                }
            }
        }
        println!(
            "rgb16_quantization target={target:?} max_absolute_transfer_error_codes={worst:.6}"
        );
    }
}

#[test]
fn rgb16_pq_uses_an_absolute_scale_at_every_peak() {
    let (device, queue, _) = crate::photographic::test_support::render_context();

    // Full-range ST 2084 RGB16 codes at known luminances, not normalized to the target.
    let anchors = [
        (0., 0),
        (0.001, 4085),
        (0.01, 9827),
        (1., 33297),
        (2.03, 38055),
        (5., 44340),
        (10., 49271),
        (20., 54225),
        (40., 59150),
    ];
    for (target, peak) in OutputTarget::HDR.into_iter().zip([5., 10., 20., 40.]) {
        let kernel = Rgb16::new(&device, target).unwrap();
        let valid: Vec<_> = anchors
            .into_iter()
            .filter(|(linear, _)| *linear <= peak)
            .collect();
        let samples: Vec<_> = valid
            .iter()
            .map(|&(linear, _)| [linear, linear, linear, 0.])
            .collect();
        for (actual, (_, code)) in decode(&run(&device, &queue, &kernel, &samples))
            .unwrap()
            .into_iter()
            .zip(valid)
        {
            assert!(
                actual
                    .into_iter()
                    .all(|channel| channel.abs_diff(code) <= 1),
                "{target:?}: {actual:?} vs {code}"
            );
        }

        // Exercise shadows, the upper end, and rounding boundaries throughout the PQ signal.
        let last = (target.encode(f64::from(peak)).unwrap() * 65535.).floor() as u16;
        let values: Vec<_> = (0..last)
            .map(|code| {
                let midpoint = target.decode((f64::from(code) + 0.5) / 65535.).unwrap() as f32;
                [
                    f32::from_bits(midpoint.to_bits() - 1),
                    midpoint,
                    f32::from_bits(midpoint.to_bits() + 1),
                    0.,
                ]
            })
            .collect();
        let mut worst = 0.0_f64;
        for block in values.chunks(16384) {
            let actual = decode(&run(&device, &queue, &kernel, block)).unwrap();
            assert_eq!(actual.len(), block.len());
            for (pixel, input) in actual.into_iter().zip(block) {
                for (code, linear) in pixel.into_iter().zip(input) {
                    let expected = target.encode(f64::from(*linear)).unwrap() * 65535.;
                    let error = (f64::from(code) - expected).abs();
                    assert!(
                        error <= 1.,
                        "{target:?}: {linear} → {code}, error {error} codes"
                    );
                    worst = worst.max(error);
                }
            }
        }
        println!(
            "rgb16_quantization target={target:?} max_absolute_transfer_error_codes={worst:.6}"
        );

        let beyond = f32::from_bits(peak.to_bits() + 1);
        assert!(matches!(
            decode(&run(&device, &queue, &kernel, &[[beyond, 0., 0., 0.]])),
            Err(Error::InvalidParameter(_))
        ));
    }
}

#[test]
fn rgb16_rejects_empty_units_and_undersized_buffers_before_encoding() {
    let (device, _queue, _renderer) = crate::photographic::test_support::render_context();
    let kernel = Rgb16::new(&device, OutputTarget::Srgb).unwrap();
    let input = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 16,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 8,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    assert!(matches!(
        kernel.encode(&device, &mut encoder, &input, &output, 0),
        Err(Error::InvalidParameter(_))
    ));
    let large_input = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 32,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    let large_output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 16,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    for (input, output) in [(&input, &large_output), (&large_input, &output)] {
        let mut encoder = device.create_command_encoder(&Default::default());
        assert!(matches!(
            kernel.encode(&device, &mut encoder, input, output, 2),
            Err(Error::InvalidParameter(_))
        ));
    }
}

#[test]
fn rgb16_rejects_invalid_components_and_range_before_cpu_publication() {
    let (device, queue, _) = crate::photographic::test_support::render_context();
    let kernel = Rgb16::new(&device, OutputTarget::Srgb).unwrap();
    for c in 0..4 {
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut pixel = [0.25, 0.5, 0.75, 0.];
            pixel[c] = value;
            assert!(matches!(
                decode(&run(&device, &queue, &kernel, &[pixel])),
                Err(Error::Gpu(_))
            ));
        }
    }
    for value in [-1., -f32::from_bits(1)] {
        assert!(matches!(
            decode(&run(&device, &queue, &kernel, &[[0., 0., 0., value]])),
            Err(Error::Gpu(_))
        ));
    }
    for value in [-f32::from_bits(1), f32::from_bits(1_f32.to_bits() + 1)] {
        assert!(matches!(
            decode(&run(&device, &queue, &kernel, &[[value, 0., 0., 0.]])),
            Err(Error::InvalidParameter(_))
        ));
    }
    let values = [[2., 0., 0., 0.], [0., 0., 0., -1.]];
    assert!(matches!(
        decode(&run(&device, &queue, &kernel, &values)),
        Err(Error::Gpu(_))
    ));
    assert!(matches!(decode(&[[0, 4 << 16]]), Err(Error::Gpu(_))));
}
