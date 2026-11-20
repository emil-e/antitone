use super::*;
use crate::editor::{Document, tests::authored_fixture};
use glam::DVec3;

fn set_map(document: &mut Document, gain: f64) -> FlatFieldSettings {
    let area = document.project.sources[0].development.active_area;
    let map = Arc::new(
        crate::flat_field::FlatField::from_nodes(area, [2, 2], vec![DVec3::splat(gain); 4])
            .unwrap(),
    );
    let settings = FlatFieldSettings::new("reference.dng", map);
    document
        .project
        .set_flat_field(Some(settings.clone()))
        .unwrap();
    settings
}

#[test]
fn accepted_maps_prepare_without_producer_registration_and_old_readers_keep_their_map() {
    let mut editor = authored_fixture(&["patches.dng"]);
    let first = set_map(&mut editor.document.state, 1.1);
    let captured = editor.correction();
    let prepared = pollster::block_on(captured.clone().acquire())
        .unwrap()
        .expect("accepted map has a GPU working copy without a restore hook");
    assert_eq!(prepared.gpu.field(), &first.map);

    let second = set_map(&mut editor.document.state, 1.2);
    let current = pollster::block_on(editor.correction().acquire())
        .unwrap()
        .unwrap();
    assert_eq!(current.gpu.field(), &second.map);
    assert!(!Arc::ptr_eq(&current, &prepared));
    assert!(Arc::ptr_eq(
        &prepared,
        &pollster::block_on(captured.acquire()).unwrap().unwrap(),
    ));
    assert_eq!(prepared.gpu.field(), &first.map);

    editor.document.project.roll.exposure = 0.5;
    assert!(Arc::ptr_eq(
        &current,
        &pollster::block_on(editor.correction().acquire())
            .unwrap()
            .unwrap(),
    ));
    editor
        .document
        .project
        .flat_field_mut()
        .unwrap()
        .reference_name = "renamed.dng".into();
    assert!(Arc::ptr_eq(
        &current,
        &pollster::block_on(editor.correction().acquire())
            .unwrap()
            .unwrap(),
    ));
    editor.document.project.flat_field_mut().unwrap().enabled = false;
    assert!(
        pollster::block_on(editor.correction().acquire())
            .unwrap()
            .is_none()
    );
    assert_eq!(current.gpu.field(), &second.map);
}

#[test]
fn document_install_needs_no_correction_restore_and_replacement_preserves_captured_readers() {
    let mut editor = authored_fixture(&["patches.dng"]);
    let mut document = editor.document.capture();
    let settings = set_map(&mut document, 1.25);
    editor.install_project(document);
    let captured = editor.correction();
    editor.new_project();
    assert!(editor.document.project.sources.is_empty());
    assert!(
        pollster::block_on(editor.correction().acquire())
            .unwrap()
            .is_none()
    );
    let prepared = pollster::block_on(captured.acquire())
        .unwrap()
        .expect("independent reader retains the installed map after document departure");
    assert_eq!(prepared.gpu.field(), &settings.map);
}
