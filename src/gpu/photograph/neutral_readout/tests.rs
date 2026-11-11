use super::*;
use crate::{
    geometry::SourceRegion,
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
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        Ok(vec![self.value; (region.width * region.height) as usize])
    }
}

#[test]
fn neutral_readout_bounds_extreme_samples_and_keeps_representable_hdr_and_floor_flags() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let region = SourceRegion::from([11, 17, 3, 2]);
    let mut input = InputSettings::negative(Calibration {
        dmin: DVec3::splat(0.7),
        dmax: DVec3::splat(0.14),
    });
    if let InputSettings::Negative { gamma, .. } = &mut input {
        *gamma = 0.125;
    }
    let matrix = DMat3::from_diagonal(DVec3::splat(1e33));
    let scalar = crate::processing::InputPipeline::new(input, matrix).unwrap();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let gate = crate::gpu::submission::SubmissionGate::default();

    for raw in [DVec3::ZERO, DVec3::new(0.07, 0.08, 0.09)] {
        let codes = (raw * 65535.).as_vec3();
        let camera = source(Arc::new(Codes {
            domain: Domain {
                bounds: Some(region.try_into().unwrap()),
                grid: Grid::axis_aligned([0.; 2], [1.; 2]),
                meaning: Meaning::CameraCodes,
            },
            value: [codes.x, codes.y, codes.z, 0.],
        }));
        let program = InputTransform::new(input, matrix).unwrap();
        let capture = Captured::new(
            CameraInput {
                matrix: program.matrix,
                field: None,
                continuous: true,
                mapping: Mapping::Originals,
            },
            Interpretation::Program(program),
        )
        .unwrap();
        let image = Assembly::captured(capture).unrepaired(camera).unwrap();
        let expected = scalar.apply(codes.as_dvec3() / 65535.).unwrap();
        let mut sum = DVec3::ZERO;
        let mut cells = 0;
        let evaluation = context
            .request(
                image.clone(),
                Region {
                    bounds: region.try_into().unwrap(),
                    grid: image.domain().grid,
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
                        assert_eq!(pixel[3] as u32, if raw == DVec3::ZERO { 7 } else { 0 });
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
        assert_eq!(cells, 6);
        let mean = sum / f64::from(cells);
        if raw == DVec3::ZERO {
            assert!(
                expected.rgb.max_element() > f64::from(f32::MAX),
                "the fixture must exercise actual f32 overflow, not ordinary HDR"
            );
            assert!(mean.is_finite() && mean.min_element() > 1.);
            assert!(
                sum.max_element() > f64::from(f32::MAX),
                "f64 statistics retain their accumulator range without encoding source pixels"
            );
        } else {
            assert!(expected.rgb.as_vec3().is_finite());
            assert!(
                (mean / expected.rgb - DVec3::ONE).abs().max_element() < 2e-5,
                "{raw:?}: {mean:?} != {:?}",
                expected.rgb
            );
        }
    }
}
