use super::*;

#[test]
fn bank_anchors_return_the_shared_scene_grey_from_the_native_response() {
    for variant in Variant::all() {
        let readout = Readout::get(variant).unwrap();
        let response = super::super::density::load(variant).unwrap();
        assert!(
            (response.sample(readout.grey_codes) - DVec3::splat(readout.scene_grey))
                .abs()
                .max_element()
                < 1e-12
        );
    }
}

#[test]
fn code_continuation_has_a_fixed_lower_bound_and_matching_upper_slope() {
    for value in [-100., -0.01, 0., 0.18, 450., 1015.] {
        assert_eq!(
            coordinates(DVec3::splat(value)).unwrap(),
            DVec3::splat(value.max(0.))
        );
    }
    for value in [1015.001, 1023., 4095., 1e9] {
        let actual = coordinates(DVec3::splat(value)).unwrap();
        assert_eq!(actual, DVec3::splat(1023. - 64. / (value - 1007.)));
        assert!(actual.max_element() < 1023.);
    }
    let h = 1e-4;
    let left = coordinates(DVec3::splat(1015. - h)).unwrap().x;
    let right = coordinates(DVec3::splat(1015. + h)).unwrap().x;
    assert!(((right - left) / (2. * h) - 1.).abs() < 1e-5);
    assert!(coordinates(DVec3::splat(f64::NAN)).is_err());
    assert!(coordinates(DVec3::splat(f64::INFINITY)).is_err());
}
