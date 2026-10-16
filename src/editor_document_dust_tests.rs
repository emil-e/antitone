use super::*;
use crate::dust_repair::{DonorChoice, RepairId, RepairOrigin, SupportMask};

#[test]
fn save_as_with_unavailable_relative_sources_adopts_the_written_baseline() {
    let directory = tempfile::tempdir().unwrap();
    let mut document = fixture(directory.path());
    let one = document.project.order[0];
    document.project.order.clear();
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
        document.project.source(one).unwrap().path,
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
fn authored_repair_state_survives_save_as_empty_roll_and_compatible_reimport() {
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
    let untouched = document.project.source(two).unwrap().clone();

    let set = &mut document.project.source_mut(one).unwrap().repairs;
    let id = set
        .insert(
            SupportMask::new([90, 30], [3, 3], vec![true; 9]).unwrap(),
            RepairOrigin::Manual,
            DonorChoice::Automatic,
        )
        .unwrap();
    let deleted = set
        .insert(
            SupportMask::new([30, 90], [2, 2], vec![true; 4]).unwrap(),
            RepairOrigin::Manual,
            DonorChoice::Offset([50, 0]),
        )
        .unwrap();
    set.remove(deleted).unwrap();

    let accepted = document.project.source(one).unwrap().repairs.clone();
    document.project.remove_from_filmstrip(one).unwrap();
    document.project.remove_from_filmstrip(two).unwrap();
    let original = directory.path().join("empty.antitone");
    let mut document = document.save(&original, Save::New).unwrap();
    assert!(!document.dirty());
    document.project.source_mut(one).unwrap().path = "one.dng".into();
    document.project.source_mut(two).unwrap().path = "two.dng".into();
    let nested = directory.path().join("nested");
    std::fs::create_dir(&nested).unwrap();
    let path = nested.join("save-as.antitone");
    let saved = document.save(&path, Save::New).unwrap();
    document.accept_save(saved);
    assert!(!document.dirty());
    assert_eq!(
        document.source_path(one).unwrap().canonicalize().unwrap(),
        first.canonicalize().unwrap()
    );

    let mut reopened = Document::open(&path).unwrap();
    assert!(reopened.project.order.is_empty());
    assert_eq!(reopened.project.source(one).unwrap().repairs, accepted);
    assert_eq!(
        reopened.project.source(two).unwrap().repairs,
        untouched.repairs
    );

    reopened.import(&[second.clone(), first.clone()]).unwrap();
    assert_eq!(reopened.project.order, [two, one]);
    assert_eq!(reopened.project.source(one).unwrap().repairs, accepted);
    assert_eq!(
        reopened.project.source(one).unwrap().repairs.repairs()[0].id,
        id
    );
    let next = reopened
        .project
        .source_mut(one)
        .unwrap()
        .repairs
        .insert(
            SupportMask::new([30, 90], [2, 2], vec![true; 4]).unwrap(),
            RepairOrigin::Manual,
            DonorChoice::Offset([50, 0]),
        )
        .unwrap();
    assert_eq!(next, RepairId(3));
    assert_eq!(
        std::fs::read(first).unwrap(),
        std::fs::read(fixture).unwrap()
    );
}
