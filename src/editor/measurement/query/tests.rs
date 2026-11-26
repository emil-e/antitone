use super::*;

#[test]
fn camera_measurements_accept_native_white_through_the_production_query() {
    let file = crate::raw::fixture::bayer_dng_with_rgb([65535; 3]);
    let editor = crate::editor::tests::authored_fixture(&[file.path().to_str().unwrap()]);
    let services = editor.source_services.as_ref().unwrap();
    let source = &editor
        .document
        .visit
        .canvas
        .ready
        .as_ref()
        .unwrap()
        .developed;
    let coverage = Coverage::rectangle(source.area());
    let samples = Samples::new(
        (&services.evaluation, &services.queue),
        source,
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
