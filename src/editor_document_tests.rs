use super::*;
use crate::{color::Balance, processing::PhotoSettings};

fn fixture(directory: &Path) -> Document {
    let mut document = Document::default();
    for name in ["one.ARW", "two.ARW"] {
        let path = directory.join(name);
        std::fs::write(&path, b"fixture source bytes").unwrap();
        document
            .register(
                Source {
                    id: SourceId(0),
                    fingerprint: Fingerprint::read(&path).unwrap(),
                    path,
                    development: SourceDevelopment {
                        dimensions: [900, 600],
                        active_area: [0, 0, 900, 600],
                        camera_to_ap0: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                        input_floor: 0.00002,
                    },
                    reference_only: false,
                    photo: PhotoSettings::default(),
                },
                Development::new(crate::raw::version(), [2., 1., 1.5, 1.]),
            )
            .unwrap();
    }
    document
}
#[test]
fn stable_identity_keeps_photo_edits_independent_of_order_and_locator() {
    let directory = tempfile::tempdir().unwrap();
    let mut document = fixture(directory.path());
    let [one, two] = document.project.order.as_slice() else {
        panic!()
    };
    let (one, two) = (*one, *two);
    let photo = PhotoSettings {
        exposure: 7.123456789012345,
        wb: Balance {
            warmth: 0.25,
            tint: -0.125,
        },
    };
    document.project.source_mut(one).unwrap().photo = photo;
    document.project.order.reverse();
    let moved = directory.path().join("renamed.ARW");
    std::fs::rename(directory.path().join("one.ARW"), &moved).unwrap();
    document.project.relink_matching(one, &moved).unwrap();
    assert_eq!(document.find_source(&moved), Some(one));
    assert_eq!(document.project.source(one).unwrap().photo, photo);
    assert_eq!(
        document.project.source(two).unwrap().photo,
        PhotoSettings::default()
    );
    assert_eq!(document.project.order, [two, one]);
}
#[test]
fn saved_snapshot_and_save_as_preserve_exact_settings_and_relative_locators() {
    let directory = tempfile::tempdir().unwrap();
    let mut document = fixture(directory.path());
    let one = document.project.order[0];
    document.project.source_mut(one).unwrap().photo.exposure = 7.123456789012345;
    assert!(document.dirty());
    let first = directory.path().join("first.antitone.json");
    let mut saved = document.save(&first, Save::New).unwrap();
    assert!(!saved.dirty());
    saved.project.source_mut(one).unwrap().path = "one.ARW".into();
    std::fs::create_dir(directory.path().join("other")).unwrap();
    let second = directory.path().join("other/second.antitone.json");
    let second_saved = saved.save(&second, Save::New).unwrap();
    assert_eq!(
        second_saved
            .source_path(one)
            .unwrap()
            .canonicalize()
            .unwrap(),
        directory.path().join("one.ARW").canonicalize().unwrap()
    );
    assert_eq!(
        second_saved.project.source(one).unwrap().path,
        Path::new("../one.ARW")
    );
    assert_eq!(Project::load(&second).unwrap(), second_saved.project);
    assert!(!second_saved.dirty());
    assert_eq!(
        second_saved.project.source(one).unwrap().photo.exposure,
        7.123456789012345
    );
    let original = std::fs::read(&first).unwrap();
    let before = saved.project.clone();
    assert!(saved.save(&first, Save::New).is_err());
    assert_eq!(std::fs::read(&first).unwrap(), original);
    assert_eq!(saved.project, before);
    assert!(
        saved
            .save(&directory.path().join("one.ARW"), Save::Replace)
            .is_err()
    );
    assert_eq!(
        std::fs::read(directory.path().join("one.ARW")).unwrap(),
        b"fixture source bytes"
    );
}
#[test]
fn input_source_collision_guard_rejects_even_a_json_named_raw() {
    let directory = tempfile::tempdir().unwrap();
    let mut document = fixture(directory.path());
    let id = document.project.order[0];
    let path = directory.path().join("source.antitone.json");
    std::fs::rename(directory.path().join("one.ARW"), &path).unwrap();
    document.project.source_mut(id).unwrap().path = path.clone();
    let before = std::fs::read(&path).unwrap();
    let error = document.save(&path, Save::Replace).err().unwrap();
    assert!(error.to_string().contains("input RAW"));
    assert_eq!(std::fs::read(&path).unwrap(), before);
}
#[cfg(unix)]
#[test]
fn relative_save_as_uses_physical_directory_bases_without_requiring_sources_to_exist() {
    let directory = tempfile::tempdir().unwrap();
    let mut document = fixture(directory.path());
    let id = document.project.order[0];
    std::fs::create_dir(directory.path().join("other")).unwrap();
    std::os::unix::fs::symlink(
        directory.path().join("other"),
        directory.path().join("alias"),
    )
    .unwrap();
    document.path = Some(directory.path().join("original.antitone.json"));
    document.project.source_mut(id).unwrap().path = "one.ARW".into();
    std::fs::remove_file(directory.path().join("one.ARW")).unwrap();
    let saved = document
        .save(
            &directory.path().join("alias/second.antitone.json"),
            Save::New,
        )
        .unwrap();
    assert_eq!(
        saved.project.source(id).unwrap().path,
        Path::new("../one.ARW")
    );
    assert!(saved.source_path(id).unwrap().ends_with("../one.ARW"));
}
#[test]
fn open_empty_document_checks_runtime_and_retains_unavailable_nonfilmstrip_records() {
    let directory = tempfile::tempdir().unwrap();
    let mut document = fixture(directory.path());
    document.project.order.clear();
    let path = directory.path().join("references.antitone.json");
    let saved = document.save(&path, Save::New).unwrap();
    std::fs::remove_file(directory.path().join("one.ARW")).unwrap();
    std::fs::remove_file(directory.path().join("two.ARW")).unwrap();
    let opened = Document::open(&path).unwrap();
    assert_eq!(opened.project, saved.project);
    assert!(!opened.dirty());
    let mut incompatible = saved.project;
    incompatible.output.contract = "different output".into();
    incompatible.save(&path, Save::Replace).unwrap();
    assert!(Document::open(&path).is_err());
}
