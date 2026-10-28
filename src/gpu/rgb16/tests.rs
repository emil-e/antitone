use super::*;

fn run(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
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
    let submission = crate::gpu::SubmissionGate::default().submit(queue, [encoder.finish()]);
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

#[test]
fn rgb16_matches_cpu_codes_and_float_neighbors_of_every_quantization_boundary() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let mut failures = 0;
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let kernel = Rgb16::new(&device, target).unwrap();
        let mut values = Vec::new();
        for code in 0..=65535 {
            let value = target.decode(f64::from(code) / 65535.).unwrap() as f32;
            values.push([value, value, value, 0.]);
            if code < 65535 {
                let value = target.decode((f64::from(code) + 0.5) / 65535.).unwrap() as f32;
                values.push([
                    f32::from_bits(value.to_bits() - 1),
                    value,
                    f32::from_bits(value.to_bits() + 1),
                    0.,
                ]);
            }
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
        // Stratified linear values and deterministic float-bit samples supplement boundary cases.
        for i in 0..65536u32 {
            let bits = i.wrapping_mul(1664525).wrapping_add(1013904223) % (1_f32.to_bits() + 1);
            values.push([
                i as f32 / 65535.,
                f32::from_bits(bits),
                (65535 - i) as f32 / 65535.,
                0.,
            ]);
        }
        let mut worst = 0;
        let mut first = None;
        for chunk in values
            .chunks((crate::gpu::execution::TILE[0] * crate::gpu::execution::TILE[1]) as usize)
        {
            let actual = decode(&run(&device, &queue, &kernel, chunk)).unwrap();
            for (pixel, &input) in actual.iter().zip(chunk) {
                for c in 0..3 {
                    let expected =
                        (target.encode(f64::from(input[c])).unwrap() * 65535.).round() as u16;
                    let difference = pixel[c].abs_diff(expected);
                    if difference != 0 {
                        failures += 1;
                        first.get_or_insert((input[c], pixel[c], expected));
                    }
                    worst = worst.max(difference);
                }
            }
        }
        println!(
            "rgb16_quantization target={target:?} max_code_difference={worst} first={first:?} cumulative_failures={failures}"
        );
    }
    assert_eq!(
        failures, 0,
        "preserve current RGB16 quantization, not just photographic tolerance"
    );
}

#[test]
fn rgb16_boundaries_and_binary_fallback_do_not_depend_on_the_float_estimate() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let target = OutputTarget::Srgb;
    let boundaries = threshold_bits(target).unwrap();
    for (index, &bits) in boundaries.iter().enumerate() {
        let expected = index as u16 + 1;
        let quantize = |bits| {
            (target.encode(f64::from(f32::from_bits(bits))).unwrap() * 65535.).round() as u16
        };
        assert_eq!(quantize(bits), expected);
        assert_eq!(quantize(bits - 1), expected - 1);
    }
    let shader = include_str!("../../shaders/rgb16.wgsl");
    let estimate = "min(u32(floor(transfer(value) * 65535.0 + 0.5)), 65535u)";
    assert_eq!(
        shader.matches(estimate).count(),
        1,
        "controlled estimate seam"
    );
    let kernel = Rgb16::from_source(&device, target, &shader.replacen(estimate, "0u", 1)).unwrap();
    let values: Vec<_> = (0..=65535)
        .map(|code| {
            let value = target.decode(f64::from(code) / 65535.).unwrap() as f32;
            [value, value, value, 0.]
        })
        .collect();
    for (part, chunk) in values.chunks(16384).enumerate() {
        let actual = decode(&run(&device, &queue, &kernel, chunk)).unwrap();
        for (i, code) in actual.iter().enumerate() {
            assert_eq!(*code, [(part * 16384 + i) as u16; 3]);
        }
    }
}

#[test]
fn rgb16_rejects_empty_oversized_and_undersized_units_before_encoding() {
    let (device, _queue, _renderer) = crate::gpu::test_support::render_context();
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
    for count in [
        0,
        crate::gpu::execution::TILE[0] * crate::gpu::execution::TILE[1] + 1,
    ] {
        let mut encoder = device.create_command_encoder(&Default::default());
        assert!(matches!(
            kernel.encode(&device, &mut encoder, &input, &output, count),
            Err(Error::InvalidParameter(_))
        ));
    }
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
    let (device, queue, _) = crate::gpu::test_support::render_context();
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
