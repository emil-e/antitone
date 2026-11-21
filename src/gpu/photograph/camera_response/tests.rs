use super::*;
use crate::processing::{Calibration, Inversion, Response};
use glam::DVec3;

struct CameraLight {
    domain: Domain,
    pixels: Vec<[f32; 4]>,
}

impl Source for CameraLight {
    fn domain(&self) -> Domain {
        self.domain
    }

    fn read(&self, region: crate::gpu::evaluation::CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        let region = region.source()?;
        assert_eq!(region, self.domain.finite().unwrap().source().unwrap());

        Ok(self.pixels.clone())
    }
}

#[test]
fn native_response_preserves_finite_zero_light_signed_values_and_continuation_flags() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let domain = Domain {
        bounds: Some([9, 13, 4, 1].into()),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Camera,
    };
    let negative = NegativeInput {
        calibration: Calibration {
            dmin: DVec3::splat(0.7),
            dmax: DVec3::splat(0.14),
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
    let readout = normalized(camera, negative).unwrap();

    assert_eq!(readout.domain().meaning, Meaning::NativePositive);
    assert_eq!(readout.domain().finite().unwrap(), domain.finite().unwrap());
    assert_eq!(readout.domain().grid, domain.grid);

    let scalar = Inversion::new(negative).unwrap();
    let request = context
        .request(
            readout,
            Region {
                bounds: domain.finite().unwrap(),
                grid: domain.grid,
            },
            Tiling { tile: [4, 1] },
        )
        .unwrap();
    let mut actual = Vec::new();

    context
        .read(
            &queue,
            &request,
            || false,
            |region, block| {
                assert_eq!(region, domain.finite().unwrap());
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
        );
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
        let flags = expected
            .floored
            .iter()
            .enumerate()
            .fold(0u32, |mask, (c, continued)| {
                mask | (u32::from(*continued) << c)
            });
        assert_eq!(actual[3] as u32, flags);
    }

    assert!(actual[0][0].is_finite() && actual[0][0] > 65536.);
    assert!(actual[1][0] > 0. && actual[1][1] < 0. && actual[1][2] < 0.);
}

#[cfg(feature = "ocio-reference")]
#[test]
fn classifier_endpoint_applies_the_provisional_grade_then_srgb_without_a_renderer() {
    let (device, queue, targets) = pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = crate::gpu::submission::request_device(&adapter, &Default::default())
            .await
            .unwrap();
        let targets = drt::Targets::new(&device, &queue).await.unwrap();
        (device, queue, targets)
    });
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let negative = NegativeInput {
        calibration: Calibration {
            dmin: DVec3::splat(0.7),
            dmax: DVec3::splat(0.14),
        },
        gamma: 0.65,
        response: Response::film(),
        balance: Default::default(),
    };
    let camera = [
        DVec3::new(0.32, 0.40, 0.51),
        DVec3::new(0.50, 0.32, 0.42),
        DVec3::new(0.40, 0.32, 0.22),
        DVec3::new(0.52, 0.62, 0.44),
    ];
    let post = glam::DMat3::from_diagonal(DVec3::new(1.8, 1.2, 0.9));
    assert!(
        classifier_image(
            [2, 2],
            &camera,
            negative,
            post,
            targets.get(OutputTarget::DisplayP3)
        )
        .is_err()
    );
    let image = classifier_image(
        [2, 2],
        &camera,
        negative,
        post,
        targets.get(OutputTarget::Srgb),
    )
    .unwrap();
    let domain = image.domain();
    assert_eq!(domain.meaning, Meaning::Display);
    let request = context
        .request(
            image,
            Region {
                bounds: domain.finite().unwrap(),
                grid: domain.grid,
            },
            Tiling { tile: [1, 1] },
        )
        .unwrap();
    let mut actual = [[0.; 3]; 4];
    context
        .read(
            &queue,
            &request,
            || false,
            |region, pixels| {
                let region = region.source()?;
                assert_eq!(pixels.len(), 1);
                actual[(region.x + 2 * region.y) as usize].copy_from_slice(&pixels[0][..3]);
                Ok(())
            },
        )
        .unwrap()
        .unwrap();

    let inversion = Inversion::new(negative).unwrap();
    let working = camera.map(|rgb| post * inversion.apply(rgb).unwrap().rgb);
    let reference = crate::rendering::CpuRenderer::new(OutputTarget::Srgb).unwrap();
    let untreated = camera.map(|rgb| inversion.apply(rgb).unwrap().rgb);
    let expected = reference.render(&working).unwrap();
    let without_post = reference.render(&untreated).unwrap();
    assert!((expected[0] - without_post[0]).abs().max_element() > 0.05);
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!(
            (glam::Vec3::from_array(actual) - expected)
                .abs()
                .max_element()
                < 3e-5
        );
    }
}
