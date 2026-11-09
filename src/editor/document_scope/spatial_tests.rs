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
                sampling: Default::default(),
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
fn automatic_regions_stay_in_memory_and_reopen_without_evidence_or_precision_fields() {
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
    let payload: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(payload.get("spatial_metadata").is_none());
    assert!(
        payload["sources"][0]["development"]
            .get("precision_floor")
            .is_none()
    );
    assert!(reopened.spatial_metadata.groups.is_empty());
    assert_eq!(*scope.spatial.ready(), regions);
    assert!(!reopened.dirty());
    let restored: DocumentScope = reopened.into();
    assert!(restored.spatial.ready().groups.is_empty());
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
    assert!(
        Document::open(&path)
            .unwrap()
            .spatial_metadata
            .groups
            .is_empty()
    );
    scope.accept_save(saved);
    assert!(!scope.dirty());
    assert_eq!(*scope.capture().spatial_metadata, second);
    scope.capture().save(&path, Save::Replace).unwrap();
    assert!(
        Document::open(&path)
            .unwrap()
            .spatial_metadata
            .groups
            .is_empty()
    );
}

#[test]
fn accepted_crop_and_analysis_area_survive_without_automatic_evidence() {
    let mut scope = authored();
    let id = scope.project.order[0];
    let [x, y, width, height] = scope.project.source(id).unwrap().development.active_area;
    let area = [x + 1, y + 1, width - 2, height - 2];
    scope.project.source_mut(id).unwrap().crop = Some(crate::crop::Crop::from_region(area.into()));
    scope.project.analysis_area = Some(area);
    let regions = metadata(&scope.capture(), 8.);
    publish(&mut scope, regions);
    let accepted = scope.project.clone();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("accepted.antitone");
    scope.capture().save(&path, Save::New).unwrap();
    let reopened = Document::open(&path).unwrap();
    assert_eq!(reopened.project, accepted);
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
    assert!(
        Document::open(&path)
            .unwrap()
            .spatial_metadata
            .groups
            .is_empty()
    );
}
