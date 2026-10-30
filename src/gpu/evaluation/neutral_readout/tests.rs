use super::*;
use crate::{
    adjustment_layers::AdjustmentLayerSnapshot,
    geometry::SourceRegion,
    gpu::ProcessingInput,
    processing::{Calibration, InputSettings},
};
use glam::{DMat3, DVec3};

struct Codes {
    domain: Domain,
    value: [f32; 4],
}

impl Source for Codes {
    fn domain(&self) -> Domain {
        self.domain
    }

    fn read(&self, region: SourceRegion) -> Result<Vec<[f32; 4]>, Error> {
        Ok(vec![self.value; (region.width * region.height) as usize])
    }
}

#[test]
fn neutral_readout_preserves_folded_range_for_f64_statistics_and_representable_local_pixels() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let region = SourceRegion::from([11, 17, 3, 2]);
    let mut input = InputSettings::negative(Calibration {
        dmin: DVec3::splat(0.7),
        dmax: DVec3::splat(0.14),
        input_floor: 1e-6,
    });
    if let InputSettings::Negative { gamma, .. } = &mut input {
        *gamma = 0.125;
    }
    let layers = AdjustmentLayerSnapshot::new(vec![]).unwrap();
    assert!(
        ProcessingInput::validate(
            &device,
            input,
            RollSettings::default(),
            &layers,
            DMat3::IDENTITY,
            region
        )
        .is_err(),
        "the fixture must need a scaled readout, not just an ordinary neutral raster"
    );
    let scalar = crate::processing::InputPipeline::new(input, DMat3::IDENTITY).unwrap();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let gate = crate::gpu::SubmissionGate::default();

    for raw in [DVec3::ZERO, DVec3::new(0.008, 0.016, 0.024)] {
        let codes = (raw * 65535.).as_vec3();
        let camera = source(Arc::new(Codes {
            domain: Domain {
                bounds: region,
                grid: Grid::axis_aligned([0.; 2], [1.; 2]),
                meaning: Meaning::CameraCodes,
            },
            value: [codes.x, codes.y, codes.z, 0.],
        }));
        let readout = Photograph::neutral_readout(
            &device,
            &renderer,
            camera,
            None,
            GpuInputProgram::new(input, DMat3::IDENTITY).unwrap(),
        )
        .unwrap();
        assert!(readout.decode_gain > 1.);
        let expected = scalar.apply(codes.as_dvec3() / 65535.).unwrap();
        let mut sum = DVec3::ZERO;
        let mut cells = 0;
        let evaluation = context
            .request(
                readout.image.clone(),
                Region {
                    bounds: region,
                    grid: readout.image.domain.grid,
                },
                Budget {
                    tile: [2, 1],
                    scratch_bytes: 1 << 20,
                },
            )
            .unwrap();
        context
            .read(
                &queue,
                &gate,
                &evaluation,
                || false,
                |_, pixels| {
                    for pixel in pixels {
                        assert!(pixel[..3].iter().all(|value| value.is_finite()));
                        assert_eq!(
                            pixel[3] as u32,
                            expected
                                .floored
                                .iter()
                                .enumerate()
                                .fold(0, |mask, (channel, floored)| mask
                                    | (u32::from(*floored) << channel))
                        );
                        sum += DVec3::new(
                            f64::from(pixel[0]),
                            f64::from(pixel[1]),
                            f64::from(pixel[2]),
                        );
                        cells += 1;
                    }
                    Ok(())
                },
            )
            .unwrap()
            .unwrap();
        let mean = sum / f64::from(cells) * readout.decode_gain;
        assert!(
            (mean / expected.rgb - DVec3::ONE).abs().max_element() < 2e-5,
            "{raw:?}: {mean:?} != {:?}",
            expected.rgb
        );

        if raw == DVec3::ZERO {
            assert!(
                expected.rgb.max_element() > f64::from(f32::MAX),
                "the f64 reduction must retain light outside a standalone f32 neutral image"
            );
        } else {
            assert!(expected.rgb.as_vec3().is_finite());
            let image = readout.decoded();
            let evaluation = context
                .request(
                    image.clone(),
                    Region {
                        bounds: region,
                        grid: image.domain.grid,
                    },
                    Budget {
                        tile: [2, 1],
                        scratch_bytes: 1 << 20,
                    },
                )
                .unwrap();
            context
                .read(
                    &queue,
                    &gate,
                    &evaluation,
                    || false,
                    |_, pixels| {
                        for pixel in pixels {
                            let actual = DVec3::new(
                                f64::from(pixel[0]),
                                f64::from(pixel[1]),
                                f64::from(pixel[2]),
                            );
                            assert!(
                                (actual / expected.rgb - DVec3::ONE).abs().max_element() < 2e-5
                            );
                        }
                        Ok(())
                    },
                )
                .unwrap()
                .unwrap();
        }
    }
}
