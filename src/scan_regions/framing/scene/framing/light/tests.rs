use super::super::tests::fixture;
use super::*;

#[test]
fn exterior_light_row_queries_match_all_pixel_centres_for_tilted_frames() {
    let (capture, _, scene) = fixture([64, 48], [8., 59., 6., 42.], Some(59.), false);
    let roles = scene.roles(&capture);
    let light = ExteriorLight::new(&capture, &roles);
    for angle in [-3., 0., 3.] {
        for right in [55.2, 58.8, 61.2] {
            let physical = PictureGeometry {
                size: capture.size,
                angle,
                sides: [Some(8.2), Some(right), Some(6.2), Some(42.2)],
            };
            let brute = (0..48).any(|y| {
                (0..64).any(|x| {
                    roles[y * 64 + x] == Role::BrighterThanBase
                        && in_hypothesis([x as f64 + 0.5, y as f64 + 0.5], &physical, None)
                })
            });
            assert_eq!(light.intersects(&physical), brute);
        }
    }
}

#[test]
fn a_one_pixel_film_cut_is_not_hidden_by_sparse_crop_or_perimeter_probes() {
    let (capture, _, scene) = fixture([64, 48], [8., 59., 6., 42.], Some(59.), false);
    let roles = scene.roles(&capture);
    let physical = PictureGeometry {
        size: capture.size,
        angle: 0.,
        sides: [Some(8.), Some(60.), Some(6.), Some(42.)],
    };
    assert!(ExteriorLight::new(&capture, &roles).intersects(&physical));
}
