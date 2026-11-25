use super::*;
use crate::acquisition::camera::ProcessingSource;
use crate::acquisition::camera::SourceGeometry;
use crate::photographic::graph::test_support::CameraFixture;
use crate::photographic::parameters::CameraMatrix;
use crate::photographic::parameters::ProcessingParameters;
use crate::photographic::test_inputs::AuthoredPhotograph;
use crate::processing::RollSettings;
use crate::{
    adjustment_layers::AdjustmentLayerSettings,
    flat_field::FlatField,
    processing::{Calibration, Response},
};
use glam::{DMat3, DVec3, Vec4};
use std::sync::Arc;
use wgpu::util::DeviceExt;

#[test]
fn corrected_conversion_and_processing_preserve_spatial_gains_and_floor_flags() {
    pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = crate::gpu::submission::request_device(&adapter, &Default::default())
            .await
            .unwrap();
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
        assert_eq!(gpu.byte_len(), field.nodes().len() as u64 * 12);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}\n{}\n{}",
                    shader(),
                    include_str!("../../parameters.wgsl"),
                    include_str!("test_read.wgsl"),
                    r#"
@group(0) @binding(1) var<storage, read_write> output: array<vec4f>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id:vec3u) {
    if id.x >= arrayLength(&scans) { return; }
    let s=convert_source(scans[id.x],id.x);
    let g=field_gains_at(source_pixel(field.source,id.x));
    output[id.x]=vec4f(s,g.x);
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
        let kernel = CameraFixture::new(&device).await.unwrap();
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
            let source = ProcessingSource::new(&device, &codes, matrix, Some(geometry)).unwrap();
            assert_eq!(source.byte_len(), codes.len() as u64 * 8);
            assert!(
                ProcessingSource::new(
                    &device,
                    &codes,
                    matrix,
                    Some(SourceGeometry::new([0, 0, 1, 1], [1, 1]).unwrap())
                )
                .is_err()
            );
            let unsupported = Arc::new(
                GpuFlatField::new(
                    &device,
                    Arc::new(
                        FlatField::from_nodes(
                            field.area(),
                            [2, 2],
                            vec![DVec3::splat(f32::MAX as f64); 4],
                        )
                        .unwrap(),
                    ),
                )
                .unwrap(),
            );
            assert!(unsupported.validate_matrix(matrix, 1.).is_err());
            let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::bytes_of(&FieldGeometry::new(&gpu, Some(geometry))),
                usage: wgpu::BufferUsages::UNIFORM,
            });
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
            let encoding = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("RGB16 source encoding for the correction primitive"),
                contents: bytemuck::bytes_of(&CameraMatrix::new(DMat3::IDENTITY).unwrap()),
                usage: wgpu::BufferUsages::UNIFORM,
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
                // The conversion primitive reads the RGB16 format flag here;
                // this entry point does not use the colour coefficients.
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: encoding.as_entire_binding(),
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
                assert!(
                    (f64::from(values[i].w) - gain.x).abs() < 1e-6,
                    "gain index={i} point={x},{y} geometry={geometry:?}"
                );
                for c in 0..3 {
                    assert!(
                        (f64::from(values[i][c]) - pixel[c]).abs() < 1e-6,
                        "index={i} channel={c} geometry={geometry:?}"
                    );
                }
            }
            let calibration = Calibration {
                dmin: DVec3::new(0.28, 0.23, 0.25),
                dmax: DVec3::new(0.01, 0.005, 0.001),
            };
            let mut cases = vec![
                (
                    ProcessingParameters::source_inspection(-16.).unwrap(),
                    None,
                    None,
                ),
                (
                    ProcessingParameters::source_inspection(0.).unwrap(),
                    None,
                    None,
                ),
                (
                    ProcessingParameters::source_inspection(16.).unwrap(),
                    None,
                    None,
                ),
            ];
            for response in [
                Response {
                    toe: 1.,
                    shoulder: 0.,
                },
                Response {
                    toe: 2.5,
                    shoulder: 0.,
                },
                Response {
                    toe: 4.,
                    shoulder: 0.003,
                },
            ] {
                let roll = RollSettings {
                    exposure: 0.5,
                    ..RollSettings::default()
                };
                let photo = AdjustmentLayerSettings {
                    exposure: -0.25,
                    ..AdjustmentLayerSettings::default()
                };
                let input = crate::processing::NegativeInput {
                    calibration,
                    response,
                    gamma: crate::processing::DEFAULT_GAMMA,
                    balance: Default::default(),
                };
                cases.push((
                    ProcessingParameters::positive(input, roll, matrix).unwrap(),
                    Some(
                        crate::adjustment_layers::AdjustmentLayerSnapshot::new(vec![
                            crate::adjustment_layers::AdjustmentLayer::global(photo),
                        ])
                        .and_then(|layers| {
                            crate::adjustment_layers::AdjustmentLayerPipeline::new(
                                input, roll, layers, matrix,
                            )
                        })
                        .unwrap(),
                    ),
                    Some(
                        AuthoredPhotograph::new(
                            &device.limits(),
                            input,
                            roll,
                            crate::adjustment_layers::AdjustmentLayerSnapshot::new(vec![
                                crate::adjustment_layers::AdjustmentLayer::global(photo),
                            ])
                            .unwrap(),
                            matrix,
                            area.into(),
                        )
                        .unwrap(),
                    ),
                ));
            }
            for (parameters, pipeline, grade) in cases {
                let mut encoder = device.create_command_encoder(&Default::default());
                if let Some(grade) = &grade {
                    kernel
                        .encode_grade(
                            &device,
                            &mut encoder,
                            &source,
                            &output,
                            &grade.clone().with_flat_field(Some(gpu.clone())),
                        )
                        .unwrap();
                } else {
                    kernel.encode_with_field(
                        &device,
                        &mut encoder,
                        &source,
                        &output,
                        parameters,
                        Some(gpu.clone()),
                    );
                }
                let bytes = codes.len() as u64 * 16;
                encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
                queue.submit([encoder.finish()]);
                let values = read(&device, &readback, bytes);
                for (pixel, value) in expected.iter().zip(values) {
                    let (reference, flags) = match &pipeline {
                        Some(p) => {
                            let r = p.apply(*pixel, [0, 0]).unwrap();
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
                                <= 2e-5 + 2e-4 * reference.abs().max_element(),
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
        }
    });
}

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
