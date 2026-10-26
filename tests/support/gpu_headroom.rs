use super::*;
use antitone::{
    flat_field::FlatField,
    gpu::{GpuFlatField, SourceGeometry},
    processing::{
        DEFAULT_GAMMA, InputSettings, MAX_CAMERA_SOURCE_CODE, MAX_SOURCE_GAIN, Response,
        RollSettings,
    },
};
use std::sync::Arc;

#[test]
fn headroom_survives_gpu_inspection_inversion_and_residual_gains_without_lut_indexing() {
    pollster::block_on(async {
        let (device, queue, kernel) = context().await;
        let pixels = [
            [65535., 65536., 131070., 0.],
            [65536., 100000.25, 70000.5, 0.],
            [MAX_CAMERA_SOURCE_CODE, 1_000_000., 123456., 0.],
        ];
        let mut source =
            ProcessingSource::new_continuous(&device, &pixels, DMat3::IDENTITY).unwrap();
        let inspected = process(
            &device,
            &queue,
            &kernel,
            &source,
            pixels.len(),
            ProcessingParameters::source(),
        );
        for (input, output) in pixels.iter().zip(inspected) {
            for c in 0..3 {
                assert_eq!(output[c], (input[c] as f64 / 65535.) as f32);
            }
        }
        for flat in [false, true] {
            let gain = if flat {
                DVec3::new(1.2, 1.5, 2.)
            } else {
                DVec3::ONE
            };
            let field = if flat {
                Some(Arc::new(
                    GpuFlatField::new(
                        &device,
                        Arc::new(
                            FlatField::from_nodes([0, 0, 3, 1], [2, 2], vec![gain; 4]).unwrap(),
                        ),
                    )
                    .unwrap(),
                ))
            } else {
                None
            };
            source
                .set_flat_field(field, SourceGeometry::new([0, 0, 3, 1], [3, 1]).unwrap())
                .unwrap();
            let calibration = Calibration {
                dmin: DVec3::splat(0.7),
                dmax: DVec3::splat(0.01),
                input_floor: CAMERA_INPUT_FLOOR * MAX_SOURCE_GAIN * gain.max_element(),
            };
            for response in [
                Response {
                    toe: 1.,
                    shoulder: 0.,
                },
                Response {
                    toe: 2.5,
                    shoulder: 0.02,
                },
            ] {
                let input = InputSettings::Negative {
                    dmin: Some(calibration.dmin),
                    dmax: Some(calibration.dmax),
                    input_floor: calibration.input_floor,
                    response,
                    gamma: DEFAULT_GAMMA,
                    balance: Default::default(),
                };
                let roll = RollSettings::default();
                let photo = PhotoSettings::default();
                let parameters =
                    ProcessingParameters::interpret(input, roll, photo, DMat3::IDENTITY).unwrap();
                let output = process(&device, &queue, &kernel, &source, pixels.len(), parameters);
                let scalar = Pipeline::new(input, roll, photo, DMat3::IDENTITY).unwrap();
                for (input, output) in pixels.iter().zip(output) {
                    let expected = scalar
                        .apply(
                            DVec3::new(input[0] as f64, input[1] as f64, input[2] as f64) / 65535.
                                * gain,
                        )
                        .unwrap();
                    for c in 0..3 {
                        assert!(
                            (output[c] as f64 - expected.rgb[c]).abs()
                                <= 2e-6 * expected.rgb.abs().max_element().max(1.),
                            "flat={flat} response={response:?} {output:?} expected={:?}",
                            expected.rgb
                        );
                    }
                    assert_eq!(output[3], 0.);
                }
            }
        }
        source.write_continuous(&queue, &pixels).unwrap();
        for invalid in [MAX_CAMERA_SOURCE_CODE + 1., f32::INFINITY, f32::NAN, -1.] {
            let mut bad = pixels;
            bad[0][0] = invalid;
            assert!(source.write_continuous(&queue, &bad).is_err());
            assert!(ProcessingSource::new_continuous(&device, &bad, DMat3::IDENTITY).is_err());
        }
    });
}
