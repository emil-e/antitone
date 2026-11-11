use super::*;
use crate::editor::tests::authored_fixture;

#[test]
fn cancelled_camera_acquisition_does_not_resolve_a_header_or_start_native_work() {
    use crate::{cancellation::Cancellation, processing::InputSettings};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let raw = crate::raw::fixture::bayer_dng_with_as_shot([1800, 2200, 2800], [2, 1, 3]);
    let mut document = Document::default();
    document.import(&[raw.path().into()]).unwrap();
    document.project.input = InputSettings::Digital;
    document.project.scan = None;
    let id = document.project.order[0];
    let physical = document.source_input(id, &Default::default()).unwrap();
    assert!(physical.development().unwrap().resolved().is_err());
    let starts = Arc::new(AtomicUsize::new(0));
    let observed = starts.clone();
    let loader = crate::source::SourceLoader::new_observed(move |_, _, _| {
        observed.fetch_add(1, Ordering::Relaxed);
        Ok(())
    });
    let cancelled = Cancellation::default();
    cancelled.cancel();
    let camera =
        pollster::block_on(SamplingInputs::default().acquire(id, &physical, &loader, &cancelled))
            .unwrap();
    assert!(camera.is_none());
    assert!(physical.development().unwrap().resolved().is_err());
    assert_eq!(starts.load(Ordering::Relaxed), 0);
    loader.close_and_wait().unwrap();
}

#[test]
fn grading_does_not_invalidate_original_inputs_but_development_changes_do() {
    let editor = authored_fixture(&["roll-0.dng", "roll-1.dng"]);
    let mut document = editor.document.capture();
    let id = document.project.order[0];
    let scan = crate::scan::Scan::open(&document.source_path(id).unwrap(), Some([1.; 4])).unwrap();
    let input = Arc::new(AnalysisInput::capture(&scan, || false).unwrap().unwrap());
    let mut inputs = SamplingInputs::default();
    inputs.insert(id, document.source_key(id).unwrap(), input.clone());
    document.project.roll.exposure = 2.;
    assert!(Arc::ptr_eq(
        &inputs.get(id, &document.source_key(id).unwrap()).unwrap(),
        &input
    ));
    document
        .project
        .source_mut(id)
        .unwrap()
        .development
        .camera_to_ap1[0][0] *= 1.1;
    assert!(inputs.get(id, &document.source_key(id).unwrap()).is_none());
    inputs.retain(&document, &Default::default());
    assert_eq!(inputs.retained_bytes(), 0);
}

#[test]
fn document_install_releases_removed_inputs_but_jobs_keep_their_own_snapshots() {
    let mut editor = authored_fixture(&["roll-0.dng", "roll-1.dng"]);
    let id = editor.document.project.order[1];
    let scan =
        crate::scan::Scan::open(&editor.document.source_path(id).unwrap(), Some([1.; 4])).unwrap();
    let input = Arc::new(AnalysisInput::capture(&scan, || false).unwrap().unwrap());
    let weak = Arc::downgrade(&input);
    let bytes = input.retained_bytes();
    let captured = editor.document.capture();
    editor
        .document
        .sampling_inputs
        .insert(id, captured.source_key(id).unwrap(), input.clone());
    assert_eq!(editor.document.sampling_inputs.retained_bytes(), bytes);
    let job = editor.document.sampling_inputs.clone();
    let mut removed = captured.clone();
    removed.project.remove_from_filmstrip(id).unwrap();
    editor.document.install(removed);
    assert_eq!(editor.document.sampling_inputs.retained_bytes(), 0);
    assert!(job.get(id, &captured.source_key(id).unwrap()).is_some());
    drop(input);
    drop(job);
    assert!(
        weak.upgrade().is_none(),
        "history/Document snapshots must not own sampling pixels"
    );
    editor.document.install(captured);
    assert_eq!(editor.document.sampling_inputs.retained_bytes(), 0);
}
