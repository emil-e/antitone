use antitone::{
    geometry::SourceRegion,
    lens::{EmbeddedLensGeometry, EmbeddedLensProfile},
};
use glam::DVec2;

#[test]
fn active_area_centres_and_valid_rectangle_preserve_pitch_without_border_replication() {
    let area = SourceRegion::from([7, 13, 40, 28]);
    let profile = EmbeddedLensProfile::from_knots(&[17. / 16.; 2], &[], &[]).unwrap();
    let geometry = EmbeddedLensGeometry::new(profile, area).unwrap();
    assert_eq!(geometry.size(), [36, 26]);
    assert_eq!(geometry.original_area(), area);
    let center = DVec2::new(26.5, 26.5);
    for [x, y] in [[0, 0], [35, 25], [12, 7], [18, 13]] {
        let base = DVec2::new((9 + x) as f64, (14 + y) as f64);
        let expected = center + (base - center) * (17. / 16.);
        for actual in geometry.sample_positions(x, y).unwrap() {
            assert_eq!(actual, expected);
            assert!(actual.x >= 7. && actual.x <= 46. && actual.y >= 13. && actual.y <= 40.);
        }
    }
    assert!(geometry.sample_positions(36, 0).is_err());
}

#[test]
fn ca_only_moves_red_blue_but_does_not_rescale_the_green_plane() {
    let area = SourceRegion::from([8, 12, 40, 28]);
    let profile =
        EmbeddedLensProfile::from_knots(&[], &[DVec2::new(129. / 128., 63. / 64.); 2], &[])
            .unwrap();
    let geometry = EmbeddedLensGeometry::new(profile, area).unwrap();
    assert_eq!(geometry.size(), [38, 26]);
    for [x, y] in [[0, 0], [37, 25], [12, 7]] {
        let points = geometry.sample_positions(x, y).unwrap();
        let base = DVec2::new((9 + x) as f64, (13 + y) as f64);
        let center = DVec2::new(27.5, 25.5);
        assert_eq!(points[1], base);
        assert_eq!(points[0], center + (base - center) * (129. / 128.));
        assert_eq!(points[2], center + (base - center) * (63. / 64.));
    }
}
