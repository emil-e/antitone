use super::*;
use crate::{
    project::Save,
    spatial_metadata::{CommonSize, Group, Metadata, Region, SourceRegions, VERSION},
};

fn metadata(document: &Document, inset: f64) -> Metadata {
    let sources: Vec<_> = document
        .spatial_inputs()
        .into_iter()
        .map(|input| {
            let [x, y, w, h] = input.development.active_area;
            let picture = Region {
                angle: 0.,
                skew: 0.,
                boundaries: [
                    [1., 0., f64::from(x) + inset],
                    [-1., 0., -(f64::from(x + w) - inset)],
                    [0., 1., f64::from(y) + inset],
                    [0., -1., -(f64::from(y + h) - inset)],
                ]
                .map(|plane| crate::spatial_metadata::Boundary::Measured { plane }),
            };
            SourceRegions {
                input,
                picture: Some(picture.clone()),
                observation: Some(picture),
                film: None,
                holder_opening: None,
                material: None,
            }
        })
        .collect();
    assert!(
        !sources.is_empty(),
        "the fixture must exercise real source-keyed data"
    );
    let value = Metadata {
        version: VERSION,
        groups: vec![Group {
            development: document.project.sources[0].development.decode.clone(),
            code_scale: document.spatial_code_scale(),
            common_size: CommonSize::fit(&sources),
            sources,
        }],
    };
    value.validate().unwrap();
    value
}

fn publish(scope: &mut DocumentScope, metadata: Metadata) {
    let request = scope.spatial.begin().unwrap();
    let inputs = scope.spatial_inputs();
    assert!(
        scope
            .spatial
            .publish(request, metadata, &scope.state.project, &inputs)
            .unwrap()
    );
}

fn authored() -> DocumentScope {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng");
    let mut document = Document::default();
    document.import(&[path]).unwrap();
    document.into()
}

#[test]
fn automatic_regions_save_and_reopen_without_entering_accepted_state_or_dirty_prompts() {
    let mut scope = authored();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("regions.antitone");
    let written = scope.capture().save(&path, Save::New).unwrap();
    scope.accept_save(written);
    let accepted = scope.project.clone();
    assert!(!scope.dirty());
    let regions = metadata(&scope.capture(), 8.);
    publish(&mut scope, regions.clone());
    assert_eq!(scope.project, accepted);
    assert!(
        !scope.dirty(),
        "automatic preparation is not an unsaved accepted edit"
    );
    assert_eq!(*scope.capture().spatial_metadata, regions);
    scope.capture().save(&path, Save::Replace).unwrap();
    let reopened = Document::open(&path).unwrap();
    assert_eq!(reopened.project, accepted);
    assert!(reopened.project.spatial_metadata.is_none());
    assert_eq!(*reopened.spatial_metadata, regions);
    assert!(!reopened.dirty());
    let restored: DocumentScope = reopened.into();
    assert_eq!(*restored.spatial.ready(), regions);
}

#[test]
fn a_captured_save_excludes_later_regions_and_completion_preserves_the_live_result() {
    let mut scope = authored();
    let first = metadata(&scope.capture(), 8.);
    publish(&mut scope, first.clone());
    let captured = scope.capture();
    let second = metadata(&scope.capture(), 12.);
    publish(&mut scope, second.clone());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("captured.antitone");
    let saved = captured.save(&path, Save::New).unwrap();
    assert_eq!(*Document::open(&path).unwrap().spatial_metadata, first);
    scope.accept_save(saved);
    assert!(!scope.dirty());
    assert_eq!(*scope.capture().spatial_metadata, second);
    scope.capture().save(&path, Save::Replace).unwrap();
    assert_eq!(*Document::open(&path).unwrap().spatial_metadata, second);
}

#[test]
fn reopening_discards_derived_data_when_the_captured_file_identity_differs() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let original =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng");
    let raw = dir.path().join("input.dng");
    std::fs::copy(original, &raw).unwrap();
    let mut document = Document::default();
    document.import(std::slice::from_ref(&raw)).unwrap();
    let mut scope: DocumentScope = document.into();
    let regions = metadata(&scope.capture(), 8.);
    publish(&mut scope, regions.clone());
    let project = scope.project.clone();
    let destination = dir.path().join("saved.antitone");
    let written = scope.capture().save(&destination, Save::New).unwrap();
    assert_eq!(*written.spatial_metadata, regions);
    std::fs::OpenOptions::new()
        .append(true)
        .open(&raw)
        .unwrap()
        .write_all(b"new capture identity before reopening")
        .unwrap();
    let reopened = Document::open(&destination).unwrap();
    assert_eq!(reopened.project, project);
    assert!(reopened.spatial_metadata.groups.is_empty());
}

#[test]
fn save_as_rebases_paths_without_rebinding_region_identity_or_replacing_publication_authority() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("original");
    let new = dir.path().join("moved-project");
    std::fs::create_dir(&old).unwrap();
    std::fs::create_dir(&new).unwrap();
    let raw = old.join("input.dng");
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        &raw,
    )
    .unwrap();
    let mut document = Document::default();
    document.import(std::slice::from_ref(&raw)).unwrap();
    let mut document = document
        .save(&old.join("original.antitone"), Save::New)
        .unwrap();
    document.project.sources[0].path = "input.dng".into();
    document.capture_source_files().unwrap();
    let mut scope: DocumentScope = document.into();
    let regions = metadata(&scope.capture(), 8.);
    publish(&mut scope, regions.clone());
    let path = new.join("save-as.antitone");
    let pending = scope.spatial.begin().unwrap();
    scope.accept_save(scope.capture().save(&path, Save::New).unwrap());
    assert!(scope.project.sources[0].path.is_relative());
    assert_ne!(
        scope.project.sources[0].path,
        std::path::Path::new("input.dng")
    );
    assert!(crate::editor::document::same_path(
        &scope.source_path(scope.project.sources[0].id).unwrap(),
        &raw
    ));
    let inputs = scope.spatial_inputs();
    assert!(
        scope
            .spatial
            .publish(pending, regions.clone(), &scope.state.project, &inputs)
            .unwrap()
    );
    assert_eq!(*scope.capture().spatial_metadata, regions);
    assert_eq!(*Document::open(&path).unwrap().spatial_metadata, regions);
}
