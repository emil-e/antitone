use super::*;

#[test]
fn compensated_arithmetic_and_polynomials_retain_precision() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let mut pairs = vec![
            (0.0001371986946930103, -2.8681616635550697, false),
            (0., 1., false),
            (1., -1., false),
            (0.1, 10., false),
            (-0.1, -10., false),
            (1. + f64::EPSILON, 1. - f64::EPSILON, false),
        ];
        let mut seed = 0x619af50108dd304bu64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for i in 0..32768 {
            let bits = next();
            let exponent = 993 + next() % 61;
            let a = f64::from_bits((bits & ((1 << 52) - 1)) | (exponent << 52));
            let other = next();
            let b = if i % 3 == 0 {
                -f64::from_bits(a.to_bits() + other % 33)
            } else {
                f64::from_bits(
                    (other & ((1 << 52) - 1)) | ((993 + next() % 61) << 52) | ((other & 1) << 63),
                )
            };
            pairs.push((a, b, false));
            pairs.push((-a, b, false));
            pairs.push((a, f64::from(b as f32), false));
            pairs.push((f64::from(a as f32), b, false));
        }
        for exponent in [-74, -73, -72, 0, 52, 80, 120] {
            for bits in [0u64, 1, (1 << 28) - 1, (1 << 51) - 1, (1 << 52) - 1] {
                let a = f64::from_bits(((exponent + 1023) as u64) << 52 | bits);
                pairs.push((a, 1., false));
                pairs.push((a, -1., false));
                pairs.push((-a, 1., false));
                if (-30..=60).contains(&exponent) {
                    pairs.push((a, -f64::from_bits(a.to_bits() - 1), false));
                }
            }
        }
        for a in [f64::from(f32::MAX), f64::from(f32::MIN_POSITIVE)] {
            for b in [1., -1.] {
                pairs.push((a, b, false));
                pairs.push((b, a, false));
            }
        }
        for (a, b) in [
            (f64::from(f32::MAX), f64::from(f32::MIN_POSITIVE)),
            (f64::from(f32::MAX), -f64::from(f32::MIN_POSITIVE)),
        ] {
            pairs.push((a, b, false));
            pairs.push((b, a, false));
        }
        for i in -500..=500 {
            pairs.push((f64::from(i) / 10., f64::from(i) / 1000., true));
        }
        for x in [
            -0.499999999999,
            -1e-20,
            -1e-16,
            0.,
            1e-16,
            1e-20,
            0.499999999999,
        ] {
            pairs.push((x, x, true));
        }
        for threshold in [0.0078125_f64, 0.015625, 0.125, 0.5] {
            for bits in [
                threshold.to_bits() - 1,
                threshold.to_bits(),
                threshold.to_bits() + 1,
            ] {
                for sign in [-1., 1.] {
                    let boundary = sign * f64::from_bits(bits);
                    for n in [-70., -16., -1., 0., 1., 16., 70.] {
                        pairs.push((n * std::f64::consts::LN_2 + boundary, boundary, true));
                    }
                    if threshold <= 0.125 {
                        let z = 2. * boundary / (1. - boundary);
                        pairs.push((boundary, z, true));
                    }
                }
            }
        }
        // Include reduction joins and dense polynomial inputs, not only ordinary source values.
        for _ in 0..16384 {
            let x = -48. + (next() >> 11) as f64 / (1u64 << 53) as f64 * 128.;
            let z = -0.5 + (next() >> 11) as f64 / (1u64 << 53) as f64;
            pairs.push((x, z, true));
        }
        for boundary in [-0.5f64, -0.25, 0.25, 0.5] {
            for shift in -4i64..=4 {
                let z = f64::from_bits((boundary.to_bits() as i64 + shift) as u64);
                if (-0.5..=0.5).contains(&z) {
                    pairs.push((z, z, true));
                }
            }
        }
        for n in -68..=115 {
            for sign in [-1., 1.] {
                let x = f64::from(n) * std::f64::consts::LN_2 + sign * std::f64::consts::LN_2 / 2.;
                for delta in [-1e-14, 0., 1e-14] {
                    pairs.push((x + delta, 0.125, true));
                }
            }
        }
        let split = |v: f64| {
            let h = v as f32;
            let l = (v - f64::from(h)) as f32;
            Vec4::new(h, l, (v - f64::from(h) - f64::from(l)) as f32, 0.)
        };
        let inputs: Vec<_> = pairs
            .iter()
            .map(|(a, b, polynomial)| {
                let mut first = split(*a);
                first.w = if *polynomial { 1. } else { 0. };
                [first, split(*b)]
            })
            .collect();
        let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("arithmetic operands"),
            contents: bytemuck::cast_slice(&inputs),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("arithmetic expansions"),
            size: pairs.len() as u64 * 48,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: output.size(),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let shader = ProcessingKernel::shader();
        let shader = if let Some(path) = std::env::var_os("ANTITONE_KERNEL_CANDIDATE_WGSL") {
            println!(
                "arithmetic candidate={}",
                std::path::Path::new(&path).display()
            );
            std::fs::read_to_string(path).unwrap()
        } else {
            shader
        };
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("compensated arithmetic gate"),
            source: wgpu::ShaderSource::Wgsl(
                [
                    &shader,
                    r#"
struct Operands { a: vec4f, b: vec4f }
@group(0) @binding(8) var<storage, read> operands: array<Operands>;
@group(0) @binding(9) var<storage, read_write> results: array<vec4f>;
@compute @workgroup_size(64)
fn arithmetic_test(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= arrayLength(&operands) { return; }
    let pair = operands[id.x];
    let a = Triple(pair.a.x, pair.a.y, pair.a.z);
    let b = Triple(pair.b.x, pair.b.y, pair.b.z);
    var first: Triple;
    var second: Triple;
    var third = number(0.0);
    if pair.a.w > 0.5 {
        first = precise_exp(a);
        second = precise_expm1(a);
        third = precise_log1p(b);
    } else {
        first = round53(add(a, b));
        second = round53(multiply(a, b));
        let pair_sum = two_sum(pair.a.x, pair.b.x);
        third = Triple(pair_sum.x, pair_sum.y, 0.0);
    }
    results[3u*id.x] = vec4f(first.h, first.l, first.t, 0.0);
    results[3u*id.x+1u] = vec4f(second.h, second.l, second.t, 0.0);
    results[3u*id.x+2u] = vec4f(third.h, third.l, third.t, 0.0);
}
"#,
                ]
                .join("\n")
                .into(),
            ),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &module,
            entry_point: Some("arithmetic_test"),
            compilation_options: Default::default(),
            cache: None,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: input.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: output.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups((pairs.len() as u32).div_ceil(64), 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, output.size());
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
        for (i, (a, b, polynomial)) in pairs.into_iter().enumerate() {
            if !polynomial {
                let (a, b) = (a as f32, b as f32);
                let sum = a + b;
                let v = sum - a;
                let u = sum - v;
                let residual = (a - u) + (b - v);
                let actual = values[3 * i + 2];
                assert_eq!(actual.x.to_bits(), sum.to_bits(), "TwoSum head: {a}, {b}");
                assert_eq!(
                    actual.y.to_bits(),
                    residual.to_bits(),
                    "TwoSum residual: {a}, {b}"
                );
                assert_eq!(actual.z, 0.);
            }
            let expected = if polynomial {
                vec![a.exp(), a.exp_m1(), b.ln_1p()]
            } else {
                vec![a + b, a * b, f64::from(a as f32) + f64::from(b as f32)]
            };
            for (j, expected) in expected.into_iter().enumerate() {
                let q = values[3 * i + j];
                let actual = f64::from(q.x) + f64::from(q.y) + f64::from(q.z);
                if !polynomial {
                    assert_eq!(actual, expected, "operation={j}: {a}, {b}");
                } else {
                    // The stored LN2 is rounded-f64; exponent-scaled range reduction
                    // and compensated evaluation still need a larger allowance than log1p.
                    // Keep n*LN2's residual until subtraction, including at large exponents.
                    let epsilon_units = if j == 2 { 8. } else { 32. };
                    assert!(
                        (actual - expected).abs() <= epsilon_units * f64::EPSILON * expected.abs(),
                        "{a}, {b}, operation={j}, polynomial={polynomial}: {actual} != {expected}"
                    );
                }
            }
        }
        drop(mapped);
        readback.unmap();
    });
}
