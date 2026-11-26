use super::*;

#[test]
fn camera_measurements_accept_native_white_through_the_production_query() {
    let file = crate::raw::fixture::bayer_dng_with_rgb([65535; 3]);
    let editor = crate::editor::tests::authored_fixture(&[file.path().to_str().unwrap()]);
    let services = editor.source_services.as_ref().unwrap();
    let mut scan = super::super::capture_scan(&editor).unwrap();
    let coverage = Coverage::rectangle(scan.area);
    let samples = Samples::new(
        (&services.evaluation, &services.queue),
        &mut scan,
        None,
        coverage,
        &|| false,
    )
    .unwrap();
    assert_eq!(
        samples
            .mean(samples.camera_light().unwrap(), &|| false)
            .unwrap(),
        DVec3::ONE
    );
}
