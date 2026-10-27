use super::*;
use crate::editor::{
    export_batch::{Draft, Kind, Scope},
    source_loader,
    tests::authored_fixture,
};
use std::sync::atomic::AtomicUsize;

#[test]
fn a_cached_renderer_failure_stops_before_acquiring_later_photos() {
    let mut editor = authored_fixture(&["roll-0.dng", "roll-1.dng"]);
    editor.set_dmin(Some(glam::DVec3::splat(0.8)));
    editor.set_dmax(Some(glam::DVec3::splat(0.02)));
    editor.sync_document();
    let directory = tempfile::tempdir().unwrap();
    let mut settings = Draft::new(
        crate::rendering::OutputTarget::Srgb,
        directory.path().to_owned(),
    );
    settings.scope = Scope::All;
    settings.kind = Kind::Exr;
    let mut request = editor.batch_input(&settings).unwrap();
    let Input::Acquire(second) = &mut request.photos[1].input else {
        panic!("second photo must exercise cold acquisition");
    };
    second.cache = None;
    let mut services = editor.export_services.take().unwrap();
    let acquisitions = Arc::new(AtomicUsize::new(0));
    let observed = acquisitions.clone();
    services.sources.developments =
        source_loader::SourceLoader::new_observed(move |_, phase, _| {
            if matches!(phase, source_loader::NativePhase::Open) {
                observed.fetch_add(1, Ordering::Relaxed);
            }
            Ok(())
        });
    let cache = Arc::new(crate::renderer::RendererCache::new(
        services.submissions.clone(),
    ));
    let injected = crate::Error::Gpu("controlled cached renderer initialization failure".into());
    cache.fail_initialization(injected.clone());
    services.renderers = cache;
    drop(editor);
    let summary = pollster::block_on(services.batch(request, Cancellation::default(), |_, _| {}));
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
        "fixture must reach the controlled shared-cache failure"
    );
    assert_eq!(
        acquisitions.load(Ordering::Relaxed),
        0,
        "later native sources must not be acquired after permanent infrastructure failure"
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}
