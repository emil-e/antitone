use super::*;
use crate::{color::Balance, processing::PhotoSettings};
#[path = "editor_document_admission_tests.rs"]
mod admission;
#[path = "editor_document_dust_tests.rs"]
mod dust;

#[test]
fn antitone_and_legacy_json_projects_open_and_save_in_place() {
    for name in [
        "grade.antitone",
        "grade.ANTITONE",
        "grade.json",
        "grade.antitone.json",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(name);
        let mut document = Document::default().save(&path, Save::New).unwrap();
        assert_eq!(document.path.as_deref(), Some(path.as_path()));
        assert!(!document.dirty());
        document.project.roll.exposure = 0.25;
        assert!(document.dirty());
        let saved = document.save(&path, Save::Replace).unwrap();
        let opened = Document::open(&path).unwrap();
        assert_eq!(opened.project, saved.project);
        assert_eq!(opened.path.as_deref(), Some(path.as_path()));
        assert!(!opened.dirty());
    }
}

#[test]
fn unsupported_project_suffixes_do_not_overwrite_the_destination() {
    let directory = tempfile::tempdir().unwrap();
    for name in ["grade", "photo.ARW", "photo.tiff"] {
        let path = directory.path().join(name);
        std::fs::write(&path, b"existing file").unwrap();
        assert!(Document::default().save(&path, Save::Replace).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"existing file");
    }
}

#[test]
fn uncalibrated_local_documents_are_durable_but_rejected_before_source_loading() {
    use crate::{
        geometry::SourcePoint,
        local_adjustments::{GradientMask, LocalAdjustment},
        project::LocalAdjustments,
    };
    let directory = tempfile::tempdir().unwrap();
    let document = fixture(directory.path());
    let path = directory.path().join("locals.json");
    let mut document = document.save(&path, Save::New).unwrap();
    let id = document.project.order[0];
    document.project.source_mut(id).unwrap().local_adjustments = Some(LocalAdjustments::default());
    let mut document = document.save(&path, Save::Replace).unwrap();
    assert!(!document.dirty());
    document
        .project
        .source_mut(id)
        .unwrap()
        .local_adjustments
        .as_mut()
        .unwrap()
        .add(
            "Disabled identity".into(),
            LocalAdjustment {
                mask: GradientMask::Linear {
                    start: SourcePoint([0., 0.]),
                    end: SourcePoint([900., 0.]),
                },
                photo: PhotoSettings::default(),
                enabled: false,
                invert: false,
                strength: 1.,
            },
        )
        .unwrap();
    assert!(document.dirty());
    let before = document.project.clone();
    let bytes = std::fs::read(&path).unwrap();
    let error = document.save(&path, Save::Replace).err().unwrap();
    assert!(error.to_string().contains("local adjustments"));
    assert_eq!(document.project, before);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    document.project.save(&path, Save::Replace).unwrap();
    let error = Document::open(&path).err().unwrap();
    assert!(error.to_string().contains("local adjustments"), "{error}");
    assert_eq!(Project::load(&path).unwrap(), before);
}

fn fixture(directory: &Path) -> Document {
    let mut document = Document::default();
    for name in ["one.ARW", "two.ARW"] {
        let path = directory.join(name);
        std::fs::write(&path, b"fixture source bytes").unwrap();
        document
            .register(
                Source {
                    id: SourceId(0),
                    path,
                    development: SourceDevelopment {
                        lens_correction: None,
                        dng_development: None,
                        dimensions: [900, 600],
                        active_area: [0, 0, 900, 600],
                        camera_model: "test-camera".into(),
                        camera_to_ap1: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                        input_floor: crate::processing::CAMERA_INPUT_FLOOR,
                    },
                    reference_only: false,
                    photo: PhotoSettings::default(),
                    local_adjustments: None,
                    repairs: Default::default(),
                    orientation: Default::default(),
                    crop: None,
                },
                Development::new(crate::raw::version(), [2., 1., 1.5, 1.]),
            )
            .unwrap();
    }
    document
}

#[test]
fn authored_dng_import_selects_as_shot_wb_and_open_preserves_saved_gains() {
    let raw_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng");
    let mut document = Document::default();
    document.import(std::slice::from_ref(&raw_path)).unwrap();
    assert_eq!(document.project.order.len(), 1);
    assert_eq!(
        document.project.development.as_ref().unwrap().fixed_wb,
        [1.; 4]
    );
    let custom = [2., 1., 1.5, 1.];
    document.project.development.as_mut().unwrap().fixed_wb = custom;
    // A saved SDK WB has a matching matrix/precision/program identity. Construct
    // that accepted basis rather than silently reinterpreting an older source record.
    let raw = RawFile::open_with_wb(&raw_path, Some(custom)).unwrap();
    let actual = source_metadata(&raw_path, &raw.metadata).unwrap();
    document.project.camera_basis = Some(actual.development.camera_basis());
    document.project.sources[0].development = actual.development;
    let directory = tempfile::tempdir().unwrap();
    let project_path = directory.path().join("grade.antitone");
    let saved = document.save(&project_path, Save::New).unwrap();
    let mut reopened = Document::open(&project_path).unwrap();
    assert_eq!(reopened.project, saved.project);
    reopened.import(std::slice::from_ref(&raw_path)).unwrap();
    assert_eq!(
        reopened.project.development.as_ref().unwrap().fixed_wb,
        custom
    );
    let source = reopened.project.order[0];
    let scan = Scan::open(&reopened.source_path(source).unwrap(), Some(custom)).unwrap();
    assert_eq!(scan.fixed_wb, custom);
    reopened.project.development.as_mut().unwrap().fixed_wb[3] = 0.;
    assert!(reopened.validate().is_err());
}

#[test]
fn file_progress_reports_only_completed_reads_including_duplicates() {
    let raw = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng");
    let directory = tempfile::tempdir().unwrap();
    let one = directory.path().join("one.dng");
    let two = directory.path().join("two.dng");
    std::fs::copy(&raw, &one).unwrap();
    std::fs::copy(&raw, &two).unwrap();
    let mut document = Document::default();
    let mut progress = Vec::new();
    document
        .import_with_progress(
            &[one.clone(), two.clone(), one.clone()],
            None,
            |done, total| progress.push((done, total)),
        )
        .unwrap();
    assert_eq!(progress, [(0, 3), (1, 3), (2, 3), (3, 3)]);
    assert_eq!(document.project.order.len(), 2);
    let project = directory.path().join("project.json");
    let saved = document.save(&project, Save::New).unwrap();
    progress.clear();
    let opened =
        Document::open_with_progress(&project, None, |done, total| progress.push((done, total)))
            .unwrap();
    assert_eq!(progress, [(0, 2), (1, 2), (2, 2)]);
    assert_eq!(opened.project, saved.project);
    std::fs::remove_file(&two).unwrap();
    progress.clear();
    assert!(
        Document::open_with_progress(&project, None, |done, total| progress.push((done, total)))
            .is_err()
    );
    assert_eq!(progress, [(0, 2), (1, 2)]);
    progress.clear();
    assert!(
        Document::default()
            .import_with_progress(&[one, two], None, |done, total| progress
                .push((done, total)))
            .is_err()
    );
    assert_eq!(progress, [(0, 2), (1, 2)]);
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
        ..PhotoSettings::default()
    };
    document.project.source_mut(one).unwrap().photo = photo;
    document.project.order.reverse();
    let moved = directory.path().join("renamed.ARW");
    std::fs::rename(directory.path().join("one.ARW"), &moved).unwrap();
    document.project.source_mut(one).unwrap().path = moved.clone();
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
    let first = directory.path().join("first.antitone");
    let mut saved = document.save(&first, Save::New).unwrap();
    assert!(!saved.dirty());
    saved.project.source_mut(one).unwrap().path = "one.ARW".into();
    std::fs::create_dir(directory.path().join("other")).unwrap();
    let second = directory.path().join("other/second.antitone");
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
fn input_source_collision_guard_rejects_even_an_antitone_named_raw() {
    let directory = tempfile::tempdir().unwrap();
    let mut document = fixture(directory.path());
    let id = document.project.order[0];
    let path = directory.path().join("source.antitone");
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

#[test]
fn oriented_imports_seed_the_photo_composition_setting() {
    use crate::geometry::{Flips, PhotoOrientation, QuarterTurns};
    use crate::raw::sony_fixture::{self, Order};
    for (tag, seed) in [
        (
            6u16,
            PhotoOrientation {
                turns: QuarterTurns::One,
                flips: Flips::default(),
            },
        ),
        (
            5,
            PhotoOrientation {
                turns: QuarterTurns::One,
                flips: Flips {
                    horizontal: true,
                    vertical: false,
                },
            },
        ),
    ] {
        let file = sony_fixture::source_oriented(Order::Little, vec![], false, false, tag);
        let mut document = Document::default();
        // The authored TIFF has no maker-note WB; declare fixed roll gains as kittests do.
        document.project.development = Some(crate::project::Development::new(
            crate::raw::version(),
            [1.; 4],
        ));
        let path = file.path().to_path_buf();
        document.import(std::slice::from_ref(&path)).unwrap();
        let id = document.project.order[0];
        assert_eq!(
            document.project.source(id).unwrap().orientation,
            seed,
            "orientation tag {tag}"
        );
    }
}
