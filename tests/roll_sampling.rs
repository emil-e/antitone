use antitone::{Error, sampling::roll_dmin};
use glam::DVec3;

const FLOOR: f64 = 1. / 65535.;

fn close(actual: DVec3, expected: DVec3) {
    assert!(
        (actual - expected).abs().max_element() < 1e-12,
        "{actual:?} != {expected:?}"
    );
}

#[test]
fn channels_select_their_own_linear_camera_transmission_band() {
    let pixels: Vec<_> = (0..=1000)
        .map(|i| {
            let x = i as f64 / 1000.;
            DVec3::new(1. + x, 2. + 2. * x, 4. - x)
        })
        .collect();
    close(
        roll_dmin(&pixels, FLOOR).unwrap(),
        DVec3::new(1.9875, 3.975, 3.9875),
    );
}

#[test]
fn mean_integrates_the_quantile_band_not_just_its_endpoints() {
    let mut pixels = vec![DVec3::ONE; 1001];
    pixels[990] = DVec3::splat(3.);
    for pixel in &mut pixels[991..] {
        *pixel = DVec3::splat(7.);
    }
    // Rank 980..995: areas 9*1 + (1+3)/2 + (3+7)/2 + 4*7 = 44.
    close(roll_dmin(&pixels, FLOOR).unwrap(), DVec3::splat(44. / 15.));
}

#[test]
fn excluded_bright_extreme_does_not_change_a_constant_film_population() {
    let mut pixels = vec![DVec3::new(0.7, 0.3, 0.17); 1001];
    pixels[1000] = DVec3::new(0.99, 0.9, 0.8);
    let original = pixels.clone();
    close(roll_dmin(&pixels, FLOOR).unwrap(), original[0]);
    assert_eq!(pixels, original);
}

#[test]
fn constant_and_singleton_pools_are_preserved() {
    let value = DVec3::new(0.7, 0.3, 0.17);
    close(roll_dmin(&[value], FLOOR).unwrap(), value);
    close(roll_dmin(&vec![value; 2048], FLOOR).unwrap(), value);
}

#[test]
fn sample_order_does_not_change_the_estimate() {
    let mut pixels: Vec<_> = (1..=1234).map(|i| DVec3::splat(i as f64 / 2000.)).collect();
    let result = roll_dmin(&pixels, FLOOR).unwrap();
    pixels.reverse();
    assert_eq!(roll_dmin(&pixels, FLOOR).unwrap(), result);
}

#[test]
fn invalid_pools_are_rejected_without_silently_filtering() {
    assert!(matches!(
        roll_dmin(&[], FLOOR),
        Err(Error::UnusableSample(_))
    ));
    for floor in [0., -1., f64::NAN, f64::INFINITY] {
        assert!(matches!(
            roll_dmin(&[DVec3::ONE], floor),
            Err(Error::InvalidParameter(_))
        ));
    }
    for value in [0., FLOOR, -1.] {
        assert!(matches!(
            roll_dmin(&[DVec3::new(1., value, 1.)], FLOOR),
            Err(Error::UnusableSample(_))
        ));
    }
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            roll_dmin(&[DVec3::new(1., 1., value)], FLOOR),
            Err(Error::NonFinitePixel { channel: 2 })
        );
    }
}

#[test]
fn averaging_finite_large_channels_does_not_overflow_intermediate_sums() {
    let value = DVec3::splat(1e300);
    let result = roll_dmin(&vec![value; 1001], FLOOR).unwrap();
    assert!(result.is_finite());
    assert!((result / value - DVec3::ONE).abs().max_element() < 1e-12);
}
