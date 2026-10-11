use super::*;
use crate::{
    flat_field::FlatField,
    processing::{Pipeline, Response},
};

#[test]
fn source_geometry_uses_exact_nearest_centres_and_rejects_overflow() {
    let geometry = SourceGeometry::new([12, 14, 8640, 5760], [257, 173]).unwrap();
    for i in [0, 1, 256, 257, 44460] {
        let [x, y] = geometry.pixel(i).unwrap();
        assert_eq!(
            x,
            12 + ((2 * u64::from(i % 257) + 1) * 8640 / (2 * 257)) as u32
        );
        assert_eq!(
            y,
            14 + ((2 * u64::from(i / 257) + 1) * 5760 / (2 * 173)) as u32
        );
    }
    assert!(geometry.pixel(44461).is_err());
    for (a, s) in [
        ([0, 0, 0, 100], [1, 1]),
        ([u32::MAX, 0, 2, 2], [1, 1]),
        ([0, 0, 8704, 6144], [u32::MAX, 1]),
        ([0, 0, 100, 100], [0, 2]),
    ] {
        assert!(SourceGeometry::new(a, s).is_err());
    }
}

#[cfg(target_os = "macos")]
#[test]
fn corrected_conversion_matches_f64_and_full_pipeline_preserves_flags_and_precision() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let matrix = DMat3::from_cols(
            DVec3::new(
                0.6062051167485374,
                0.01610845369283179,
                -0.003058878537448056,
            ),
            DVec3::new(0.38024835632147796, 1.192683246450255, -0.14899261591291027),
            DVec3::new(
                0.013546559132378455,
                -0.20879153479206025,
                1.1520515014561068,
            ),
        );
        let (w, h) = (17, 11);
        let nodes = (0..h)
            .flat_map(|y| {
                (0..w).map(move |x| {
                    DVec3::new(
                        0.93 + 0.3 * (x * x) as f64 / (w * w) as f64,
                        1.02 + 0.25 * y as f64 / h as f64,
                        0.88 + 0.35 * (x + y) as f64 / (w + h) as f64,
                    )
                })
            })
            .collect();
        let field = Arc::new(FlatField::from_nodes([46, 46, 8602, 5508], [w, h], nodes).unwrap());
        let gpu = Arc::new(GpuFlatField::new(&device, field.clone()).unwrap());
        assert_eq!(gpu.byte_len(), field.nodes().len() as u64 * 36);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}\n{}",
                    include_str!("shaders/camera_conversion.wgsl"),
                    include_str!("shaders/flat_field.wgsl"),
                    r#"
@group(0) @binding(0) var<storage, read> scans: array<vec2u>;
@group(0) @binding(1) var<storage, read_write> output: array<vec4f>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id:vec3u) {
    if id.x >= arrayLength(&scans) { return; }
    let s=convert_source(scans[id.x],id.x);
    let g=field_gains(id.x);
    output[3u*id.x]=vec4f(s.high,g[0].h);
    output[3u*id.x+1u]=vec4f(s.low,g[0].l);
    output[3u*id.x+2u]=vec4f(s.tail,g[0].t);
}
"#
                )
                .into(),
            ),
        });
        let conversion = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let kernel = ProcessingKernel::new(&device).await.unwrap();
        for (area, size) in [
            ([12, 12, 8640, 5760], [257, 173]),
            ([4000, 2600, 64, 32], [64, 32]),
            ([40, 40, 96, 96], [96, 96]),
            ([8600, 5500, 96, 96], [96, 96]),
        ] {
            let geometry = SourceGeometry::new(area, size).unwrap();
            let mut seed = 0x913048feu32;
            let mut codes: Vec<_> = (0..size[0] * size[1])
                .map(|_| {
                    let mut pixel = [0, 0, 0, 65535];
                    for code in &mut pixel[..3] {
                        seed ^= seed << 13;
                        seed ^= seed >> 17;
                        seed ^= seed << 5;
                        *code = seed as u16;
                    }
                    pixel
                })
                .collect();
            codes[0] = [0, 0, 0, 124];
            codes[1] = [65535, 65535, 65535, 0];
            codes[2] = [65535, 0, 0, 42];
            let mut source = ProcessingSource::new(&device, &codes, matrix).unwrap();
            source.set_flat_field(Some(gpu.clone()), geometry).unwrap();
            assert_eq!(source.byte_len(), codes.len() as u64 * 8);
            assert!(
                source
                    .set_flat_field(None, SourceGeometry::new([0, 0, 1, 1], [1, 1]).unwrap())
                    .is_err()
            );
            assert!(source.flat_field.is_some());
            for value in [2. * f32::MIN_POSITIVE as f64, f32::MAX as f64] {
                let unsupported = Arc::new(
                    FlatField::from_nodes(field.area(), [2, 2], vec![DVec3::splat(value); 4])
                        .unwrap(),
                );
                let unsupported = Arc::new(GpuFlatField::new(&device, unsupported).unwrap());
                assert!(source.set_flat_field(Some(unsupported), geometry).is_err());
                assert!(Arc::ptr_eq(&source.flat_field.as_ref().unwrap().0, &gpu));
            }
            let tiny = Arc::new(
                FlatField::from_nodes(
                    field.area(),
                    [2, 2],
                    vec![DVec3::splat(1e-35_f32 as f64); 4],
                )
                .unwrap(),
            );
            let tiny = Arc::new(GpuFlatField::new(&device, tiny).unwrap());
            let mut amplified =
                ProcessingSource::new(&device, &codes, DMat3::from_diagonal(DVec3::splat(1e10)))
                    .unwrap();
            assert!(amplified.set_flat_field(Some(tiny), geometry).is_err());
            assert!(amplified.flat_field.is_none());
            let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::bytes_of(&flat::FieldGeometry::new(&gpu, geometry)),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let output = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: codes.len() as u64 * 48,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: output.size(),
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let entries = [
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: source.input.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: gpu.nodes.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: uniform.as_entire_binding(),
                },
            ];
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &conversion.get_bind_group_layout(0),
                entries: &entries,
            });
            let expected: Vec<_> = codes
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let [x, y] = geometry.pixel(i as u32).unwrap();
                    DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64) / 65535.
                        * field.gain([x as f64 + 0.5, y as f64 + 0.5]).unwrap()
                })
                .collect();
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&conversion);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups((codes.len() as u32).div_ceil(64), 1, 1);
            }
            encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, output.size());
            queue.submit([encoder.finish()]);
            let values = read(&device, &readback, output.size());
            for (i, pixel) in expected.iter().enumerate() {
                let [x, y] = geometry.pixel(i as u32).unwrap();
                let gain = field.gain([x as f64 + 0.5, y as f64 + 0.5]).unwrap();
                let actual_gain = values[3 * i].w as f64
                    + values[3 * i + 1].w as f64
                    + values[3 * i + 2].w as f64;
                assert_eq!(
                    actual_gain.to_bits(),
                    gain.x.to_bits(),
                    "gain index={i} point={x},{y} geometry={geometry:?}"
                );
                for c in 0..3 {
                    let value = values[3 * i][c] as f64
                        + values[3 * i + 1][c] as f64
                        + values[3 * i + 2][c] as f64;
                    assert_eq!(
                        value.to_bits(),
                        pixel[c].to_bits(),
                        "index={i} channel={c} geometry={geometry:?}"
                    );
                }
            }
            let floor = field.precision_floor().unwrap();
            let calibration = Calibration {
                dmin: DVec3::new(0.28, 0.23, 0.25),
                dmax: DVec3::new(0.01, 0.005, 0.001),
                input_floor: floor,
            };
            let mut cases = vec![
                (ProcessingParameters::source_inspection(-16.).unwrap(), None),
                (ProcessingParameters::source_inspection(0.).unwrap(), None),
                (ProcessingParameters::source_inspection(16.).unwrap(), None),
            ];
            for response in [
                Response::Original,
                Response::Film {
                    toe: 2.5,
                    shoulder: 0.,
                },
                Response::Film {
                    toe: 4.,
                    shoulder: 0.003,
                },
            ] {
                let roll = RollSettings {
                    response,
                    exposure: 2.,
                    ..RollSettings::default()
                };
                let photo = PhotoSettings {
                    exposure: 3.,
                    ..PhotoSettings::default()
                };
                cases.push((
                    ProcessingParameters::positive(calibration, roll, photo, matrix).unwrap(),
                    Some(Pipeline::new(calibration, roll, photo, matrix).unwrap()),
                ));
            }
            for offset in [0., 1e-12, -1e-12] {
                let anchored = Calibration {
                    dmin: expected[1] * (1. + offset),
                    ..calibration
                };
                let roll = RollSettings::new_grade();
                cases.push((
                    ProcessingParameters::positive(
                        anchored,
                        roll,
                        PhotoSettings::default(),
                        matrix,
                    )
                    .unwrap(),
                    Some(Pipeline::new(anchored, roll, PhotoSettings::default(), matrix).unwrap()),
                ));
            }
            for (parameters, pipeline) in cases {
                let mut encoder = device.create_command_encoder(&Default::default());
                kernel.encode(&device, &mut encoder, &source, &output, parameters);
                let bytes = codes.len() as u64 * 16;
                encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
                queue.submit([encoder.finish()]);
                let values = read(&device, &readback, bytes);
                for (pixel, value) in expected.iter().zip(values) {
                    let (reference, flags) = match &pipeline {
                        Some(p) => {
                            let r = p.apply(*pixel).unwrap();
                            (r.rgb, r.floored)
                        }
                        None => (
                            matrix
                                * pixel
                                * (parameters.post[0].x / ProcessingParameters::source().post[0].x)
                                    as f64,
                            [false; 3],
                        ),
                    };
                    for c in 0..3 {
                        assert!(
                            (value[c] as f64 - reference[c]).abs()
                                <= 2e-5 + 2e-4 * reference[c].abs(),
                            "actual={value:?} expected={reference:?}"
                        );
                    }
                    let mask = flags
                        .iter()
                        .enumerate()
                        .fold(0u32, |m, (c, f)| m | ((*f as u32) << c));
                    assert_eq!(value.w, mask as f32);
                }
            }
            source.set_flat_field(None, geometry).unwrap();
            assert!(source.flat_field.is_none());
        }
    });
}
#[cfg(target_os = "macos")]
fn read(device: &wgpu::Device, buffer: &wgpu::Buffer, bytes: u64) -> Vec<Vec4> {
    let (send, receive) = std::sync::mpsc::channel();
    buffer
        .slice(..bytes)
        .map_async(wgpu::MapMode::Read, move |r| send.send(r).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    receive.recv().unwrap().unwrap();
    let mapped = buffer.slice(..bytes).get_mapped_range().unwrap();
    let values = bytemuck::cast_slice(&mapped).to_vec();
    drop(mapped);
    buffer.unmap();
    values
}
