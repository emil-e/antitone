use super::*;
use crate::dust_repair::{RepairId, RepairOrigin, SupportMask};

#[test]
fn save_as_with_unavailable_relative_flat_field_reference_adopts_the_written_baseline() {
    let directory = tempfile::tempdir().unwrap();
    let document = fixture(directory.path());
    let reference = document.project.sources[0].clone();
    let map = std::sync::Arc::new(
        crate::flat_field::FlatField::from_nodes(
            reference.development.active_area,
            [2, 2],
            vec![glam::DVec3::ONE; 4],
        )
        .unwrap(),
    );
    let mut document = document;
    document
        .project
        .set_flat_field(Some(crate::project::FlatFieldSettings::new(
            &reference, map,
        )));
    let other = document.project.order[1];
    document.project.remove_from_filmstrip(other).unwrap();
    document
        .project
        .remove_from_filmstrip(reference.id)
        .unwrap();
    assert!(document.project.order.is_empty());
    assert_eq!(document.project.sources.len(), 1);
    let original = directory.path().join("project.antitone");
    let mut document = document.save(&original, Save::New).unwrap();
    for source in &mut document.project.sources {
        source.path = PathBuf::from(source.path.file_name().unwrap());
    }
    let mut document = document.save(&original, Save::Replace).unwrap();
    for name in ["one.ARW", "two.ARW"] {
        std::fs::remove_file(directory.path().join(name)).unwrap();
    }

    let nested = directory.path().join("nested");
    std::fs::create_dir(&nested).unwrap();
    let saved = document
        .save(&nested.join("project.antitone"), Save::New)
        .unwrap();
    document.accept_save(saved);
    assert!(!document.dirty());
    assert_eq!(
        document.project.source(reference.id).unwrap().path,
        PathBuf::from("../one.ARW")
    );
    assert_eq!(
        Document::open(document.path.as_deref().unwrap())
            .unwrap()
            .project,
        document.project
    );
}

#[test]
fn authored_repairs_are_deleted_with_their_photo_and_reimport_starts_clean() {
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("one.dng");
    let second = directory.path().join("two.dng");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng");
    for path in [&first, &second] {
        std::fs::copy(&fixture, path).unwrap();
    }

    let mut document = Document::default();
    document.import(&[first.clone(), second.clone()]).unwrap();
    let one = document.project.order[0];
    let two = document.project.order[1];
    let untouched = document.project.source(two).unwrap().repairs.clone();

    let set = &mut document.project.source_mut(one).unwrap().repairs;
    let id = set
        .insert(
            SupportMask::new([90, 30], [3, 3], vec![true; 9]).unwrap(),
            RepairOrigin::Manual,
            [64, 0],
        )
        .unwrap();
    let deleted = set
        .insert(
            SupportMask::new([30, 90], [2, 2], vec![true; 4]).unwrap(),
            RepairOrigin::Manual,
            [50, 0],
        )
        .unwrap();
    set.remove(deleted).unwrap();
    assert_eq!(set.repairs()[0].id, id);

    document.project.remove_from_filmstrip(one).unwrap();
    assert!(document.project.source(one).is_err());
    assert_eq!(document.project.source(two).unwrap().repairs, untouched);
    document.project.remove_from_filmstrip(two).unwrap();
    assert!(document.project.source(two).is_err());

    let original = directory.path().join("empty.antitone");
    let document = document.save(&original, Save::New).unwrap();
    assert!(!document.dirty());
    let mut reopened = Document::open(&original).unwrap();
    assert!(reopened.project.order.is_empty());
    assert!(reopened.project.sources.is_empty());

    reopened.import(&[second.clone(), first.clone()]).unwrap();
    assert_eq!(reopened.project.order.len(), 2);
    let new_one = reopened.find_source(&first).unwrap();
    let new_two = reopened.find_source(&second).unwrap();
    assert_ne!(new_one, one);
    assert_ne!(new_two, two);
    assert!(
        reopened
            .project
            .source(new_one)
            .unwrap()
            .repairs
            .repairs()
            .is_empty()
    );
    assert!(
        reopened
            .project
            .source(new_two)
            .unwrap()
            .repairs
            .repairs()
            .is_empty()
    );
    let next = reopened
        .project
        .source_mut(new_one)
        .unwrap()
        .repairs
        .insert(
            SupportMask::new([30, 90], [2, 2], vec![true; 4]).unwrap(),
            RepairOrigin::Manual,
            [50, 0],
        )
        .unwrap();
    assert_eq!(next, RepairId(1));
    assert_eq!(
        std::fs::read(first).unwrap(),
        std::fs::read(fixture).unwrap()
    );
}
