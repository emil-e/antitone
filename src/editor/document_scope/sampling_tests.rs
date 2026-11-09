use super::*;
use crate::editor::tests::authored_fixture;

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
        .decode
        .white_balance[0] = 2.;
    assert!(inputs.get(id, &document.source_key(id).unwrap()).is_none());
    inputs.retain(&document);
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
