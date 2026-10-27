use super::*;
use crate::{
    adjustment_layers::{AdjustmentLayer, AdjustmentLayerSettings, AdjustmentLayerSnapshot},
    color::Balance,
    geometry::SourceRegion,
    processing::{Calibration, NegativeInput, Response},
};

#[test]
fn one_photographic_grade_runs_on_independently_resolved_digital_and_negative_inputs() {
    pollster::block_on(async {
        let adapter = instance()
            .request_adapter(&Default::default())
            .await
            .unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let kernel = ProcessingKernel::new(&device).await.unwrap();
        let area = SourceRegion::from([0, 0, 2, 1]);
        let roll = RollSettings {
            exposure: 0.5,
            ..Default::default()
        };
        let layers =
            AdjustmentLayerSnapshot::new(vec![AdjustmentLayer::global(AdjustmentLayerSettings {
                exposure: 0.25,
                ..Default::default()
            })])
            .unwrap();
        let grade = Arc::new(GpuGrade::new(&device, roll, layers, area).unwrap());
        let negative = InputSettings::from(NegativeInput {
            calibration: Calibration {
                dmin: DVec3::splat(0.8),
                dmax: DVec3::splat(0.02),
                input_floor: 1. / 65535.,
            },
            response: Response {
                toe: 1.,
                shoulder: 0.,
            },
            gamma: 1.,
            balance: Balance::default(),
        });
        let pixels = [[0.9_f32, 0.015, 0.11, 0.], [0.04, 0.3, 0.21, 0.]]
            .map(|p| [p[0] * 65535., p[1] * 65535., p[2] * 65535., 0.]);
        let bytes = (pixels.len() * 16) as u64;
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut saw_signed = false;
        let mut saw_hdr = false;
        for input in [InputSettings::Digital, negative] {
            for matrix in [
                DMat3::from_diagonal(DVec3::new(0.8, 1.1, 0.7)),
                DMat3::from_cols(
                    DVec3::new(1., 0., -0.2),
                    DVec3::new(-0.3, 1., 0.),
                    DVec3::new(0., 0., 1.5),
                ),
            ] {
                let cases = [
                    (SourceGeometry::new([0, 0, 2, 1], [2, 1]).unwrap(), area),
                    #[cfg(all(feature = "raw", feature = "export"))]
                    (
                        SourceGeometry::original_pair([[4, 4], [5, 5]], [1, 1]).unwrap(),
                        [0, 0, 8, 8].into(),
                    ),
                ];
                for (geometry, source_area) in cases {
                    let program = GpuInputProgram::new(input, matrix).unwrap();
                    let mut source = program.source_continuous(&device, &pixels).unwrap();
                    source.set_geometry(geometry).unwrap();
                    let pipeline =
                        GpuPipeline::compose(&device, program, grade.clone(), source_area).unwrap();
                    let mut encoder = device.create_command_encoder(&Default::default());
                    kernel
                        .encode_grade(&device, &mut encoder, &source, &output, &pipeline)
                        .unwrap();
                    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
                    let submission = queue.submit([encoder.finish()]);
                    let (tx, rx) = std::sync::mpsc::channel();
                    readback
                        .slice(..)
                        .map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
                    device
                        .poll(wgpu::PollType::Wait {
                            submission_index: Some(submission),
                            timeout: Some(std::time::Duration::from_secs(10)),
                        })
                        .unwrap();
                    rx.recv_timeout(std::time::Duration::from_secs(2))
                        .unwrap()
                        .unwrap();
                    {
                        let mapped = readback.slice(..).get_mapped_range().unwrap();
                        let actual: &[[f32; 4]] = bytemuck::cast_slice(&mapped);
                        for (actual, code) in actual.iter().zip(pixels) {
                            let camera = DVec3::new(
                                f64::from(code[0]),
                                f64::from(code[1]),
                                f64::from(code[2]),
                            ) / 65535.;
                            let positive = if input.is_negative() {
                                camera.map(|v| (0.8 / v - 1.) / (0.8 / 0.02 - 1.))
                            } else {
                                camera
                            };
                            let expected = matrix * positive * 0.75_f64.exp2();
                            saw_signed |= expected.min_element() < 0.;
                            saw_hdr |= expected.max_element() > 1.;
                            for c in 0..3 {
                                assert!(
                                    (f64::from(actual[c]) - expected[c]).abs()
                                        < 3e-5 + 3e-5 * expected[c].abs(),
                                    "{input:?} channel{c}: {actual:?} vs {expected:?}"
                                );
                            }
                        }
                    }
                    readback.unmap();
                    #[cfg(all(feature = "raw", feature = "export"))]
                    if geometry.is_pair() {
                        source
                            .set_geometry(
                                SourceGeometry::original_pair(
                                    [[0, 0], [source_area.width, 0]],
                                    [1, 1],
                                )
                                .unwrap(),
                            )
                            .unwrap();
                        assert!(
                            pipeline.require_source(&source).is_err(),
                            "pairs must remain within the actual composed source, not the grade's old extent"
                        );
                    }
                }
            }
        }
        assert!(
            saw_signed && saw_hdr,
            "controls must exercise signed and highlight-headroom working values"
        );
    });
}
