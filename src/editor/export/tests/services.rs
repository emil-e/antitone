use super::*;
use crate::editor::{
    export::{Draft, Kind, Scope},
    source_loading,
    tests::authored_fixture,
};
use std::sync::atomic::AtomicUsize;

#[test]
fn a_display_initialization_failure_stops_tiff_before_acquiring_later_photos() {
    let mut editor = authored_fixture(&["roll-0.dng", "roll-1.dng"]);
    editor.set_dmin(Some(glam::DVec3::splat(0.8)));
    editor.set_dmax(Some(glam::DVec3::splat(0.02)));

    let directory = tempfile::tempdir().unwrap();
    let mut settings = Draft::new(
        crate::rendering::OutputTarget::Srgb,
        directory.path().to_owned(),
    );
    settings.scope = Scope::All;
    settings.kind = Kind::Tiff;
    let mut request = editor.capture_export(&settings).unwrap();
    let Input::Acquire(second) = &mut request.photos[1].input else {
        panic!("second photo must exercise cold acquisition");
    };
    second.sources = None;
    let mut services = editor.output_services.take().unwrap();
    let acquisitions = Arc::new(AtomicUsize::new(0));
    let observed = acquisitions.clone();
    services.sources.developments =
        source_loading::SourceLoader::new_observed(move |_, phase, _| {
            if matches!(phase, source_loading::NativePhase::Open) {
                observed.fetch_add(1, Ordering::Relaxed);
            }
            Ok(())
        });
    let injected = crate::Error::Gpu("controlled display initialization failure".into());
    services.display = crate::editor::display::Warmup::failed(injected.clone());
    drop(editor);
    let summary =
        pollster::block_on(services.export_batch(request, Cancellation::default(), |_, _| {}));
    assert!(summary.stopped);
    assert_eq!(
        (summary.failed(), summary.exported(), summary.unattempted()),
        (1, 0, 1)
    );
    let Outcome::Failed { error, .. } = &summary.outcomes[0] else {
        panic!("shared initialization must fail the current file");
    };
    assert_eq!(
        error,
        &injected.to_string(),
        "fixture must reach the controlled target initialization failure"
    );
    assert_eq!(
        acquisitions.load(Ordering::Relaxed),
        0,
        "later native sources must not be acquired after permanent infrastructure failure"
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn scene_export_can_publish_without_display_targets() {
    let mut editor = authored_fixture(&["roll-0.dng"]);
    editor.set_dmin(Some(glam::DVec3::splat(0.8)));
    editor.set_dmax(Some(glam::DVec3::splat(0.02)));
    let directory = tempfile::tempdir().unwrap();
    let mut settings = Draft::new(
        crate::rendering::OutputTarget::Srgb,
        directory.path().to_owned(),
    );
    settings.kind = Kind::Exr;
    let request = editor.capture_export(&settings).unwrap();
    let mut services = editor.output_services.take().unwrap();
    services.display =
        crate::editor::display::Warmup::failed(crate::Error::Gpu("display unavailable".into()));
    let summary =
        pollster::block_on(services.export_batch(request, Cancellation::default(), |_, _| {}));
    assert!(!summary.stopped);
    let [Outcome::Exported(path)] = summary.outcomes.as_slice() else {
        panic!("scene export must publish a file even without display targets");
    };
    exr::prelude::read_first_flat_layer_from_file(path).unwrap();
}
