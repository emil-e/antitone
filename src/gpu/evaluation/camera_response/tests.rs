use super::*;
use crate::{
    geometry::SourceRegion,
    processing::{Calibration, Inversion, Response},
};
use glam::DVec3;

struct CameraLight {
    domain: Domain,
    pixels: Vec<[f32; 4]>,
}

impl Source for CameraLight {
    fn domain(&self) -> Domain {
        self.domain
    }

    fn read(&self, region: SourceRegion) -> Result<Vec<[f32; 4]>, Error> {
        assert_eq!(region, self.domain.bounds);

        Ok(self.pixels.clone())
    }
}

#[test]
fn native_response_preserves_floor_flags_continuations_and_folded_f64_range() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let gate = crate::gpu::SubmissionGate::default();
    let domain = Domain {
        bounds: [9, 13, 4, 1].into(),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Camera,
    };
    let negative = NegativeInput {
        calibration: Calibration {
            dmin: DVec3::splat(0.7),
            dmax: DVec3::splat(0.14),
            input_floor: 1e-6,
        },
        gamma: 0.125,
        response: Response::film(),
        balance: Default::default(),
    };
    let base = negative.calibration.dmin.x as f32;
    let pixels = vec![
        [0., 0.01, 0.02, 0.],
        [base.next_down(), base.next_up(), 0.8, 0.],
        [0.008, 0.016, 0.024, 0.],
        [1., 2., 3., 0.],
    ];
    let camera = source(Arc::new(CameraLight {
        domain,
        pixels: pixels.clone(),
    }));
    let readout = CameraReadout::normalized(camera, negative).unwrap();

    assert_eq!(readout.image.domain.meaning, Meaning::NativePositive);
    assert_eq!(readout.image.domain.bounds, domain.bounds);
    assert_eq!(readout.image.domain.grid, domain.grid);
    assert!(
        readout.decode_gain > 1.,
        "the endpoint must need folded range"
    );

    let scalar = Inversion::new(negative).unwrap();
    let request = context
        .request(
            readout.image,
            Region {
                bounds: domain.bounds,
                grid: domain.grid,
            },
            Budget {
                tile: [4, 1],
                scratch_bytes: 1 << 20,
            },
        )
        .unwrap();
    let mut actual = Vec::new();

    context
        .read(
            &queue,
            &gate,
            &request,
            || false,
            |region, block| {
                assert_eq!(region, domain.bounds);
                actual.extend_from_slice(block);

                Ok(())
            },
        )
        .unwrap()
        .unwrap();

    assert_eq!(actual.len(), pixels.len());

    for (index, (actual, input)) in actual.iter().zip(pixels).enumerate() {
        let input = DVec3::new(
            f64::from(input[0]),
            f64::from(input[1]),
            f64::from(input[2]),
        );
        let expected = scalar.apply(input).unwrap();
        let decoded = DVec3::new(
            f64::from(actual[0]),
            f64::from(actual[1]),
            f64::from(actual[2]),
        ) * readout.decode_gain;
        let mut tolerance = expected.rgb.abs().max(DVec3::splat(1e-6)) * 2e-5;

        if index == 1 {
            // Relative error is ill-conditioned at film base. Bound two f32 input steps
            // through the independent curve; sign checks below still distinguish both sides.
            let step = input.abs() * (2. * f64::from(f32::EPSILON));
            let lower = scalar.apply(input - step).unwrap().rgb;
            let upper = scalar.apply(input + step).unwrap().rgb;
            tolerance = tolerance.max((upper - lower).abs());
        }

        assert!(
            ((decoded - expected.rgb).abs() / tolerance).max_element() < 1.,
            "{index}: {decoded:?} != {:?}",
            expected.rgb
        );
        assert_eq!(actual[3] as u32, if index == 0 { 1 } else { 0 });
    }

    assert!(f64::from(actual[0][0]) * readout.decode_gain > f64::from(f32::MAX));
    assert!(actual[1][0] > 0. && actual[1][1] < 0. && actual[1][2] < 0.);
}
