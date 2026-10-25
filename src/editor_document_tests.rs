use super::*;
use crate::project::Development;
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
            .register_with_development(
                Source {
                    id: SourceId(0),
                    path,
                    development: SourceDevelopment {
                        decode: Development::new("test-backend".into(), [1.; 4]),
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
                Development::new(crate::raw::version(), [1.; 4]),
            )
            .unwrap();
    }
    document
}

#[test]
fn stage_wb_policy_selects_unity_for_negative_and_as_shot_for_digital() {
    let original =
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"))
            .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let raw_path = directory.path().join("red-neutral.dng");
    std::fs::write(
        &raw_path,
        super::super::tests::dng_with_red_as_shot(&original),
    )
    .unwrap();
    let as_shot = read_source(&raw_path, None)
        .unwrap()
        .development
        .decode
        .fixed_wb;
    assert_ne!(as_shot, crate::project::UNITY_WB);

    let mut negative = Document::default();
    negative.import(std::slice::from_ref(&raw_path)).unwrap();
    assert_eq!(
        negative.project.development().unwrap().fixed_wb,
        crate::project::UNITY_WB
    );

    let mut digital = Document::default();
    digital.project.input = crate::processing::InputSettings::Digital;
    digital.import(std::slice::from_ref(&raw_path)).unwrap();
    assert_eq!(digital.project.development().unwrap().fixed_wb, as_shot);
}

#[test]
fn mixed_as_shot_wb_does_not_select_digital_or_change_the_negative_basis() {
    let original =
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"))
            .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first.dng");
    let red = directory.path().join("red-neutral.dng");
    std::fs::write(&first, &original).unwrap();
    std::fs::write(&red, super::super::tests::dng_with_red_as_shot(&original)).unwrap();
    let mut document = Document::default();
    document.import(&[first, red]).unwrap();
    assert!(document.project.input.is_negative());
    assert_eq!(document.project.sources.len(), 2);
    assert!(
        document
            .project
            .sources
            .iter()
            .all(|source| source.development.decode.fixed_wb == crate::project::UNITY_WB)
    );
    assert!(document.project.require_scan_sources().is_ok());
}

#[test]
fn stage_redevelopment_skips_unavailable_removed_sources_and_reactivates_available_ones() {
    let original =
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"))
            .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first.dng");
    let missing = directory.path().join("missing.dng");
    let removed = directory.path().join("removed.dng");
    std::fs::write(&first, &original).unwrap();
    std::fs::write(
        &missing,
        super::super::tests::dng_with_red_as_shot(&original),
    )
    .unwrap();
    std::fs::write(
        &removed,
        super::super::tests::dng_with_red_as_shot(&original),
    )
    .unwrap();
    let mut document = Document::default();
    document
        .import(&[first.clone(), missing.clone(), removed.clone()])
        .unwrap();
    let missing_id = document.find_source(&missing).unwrap();
    let removed_id = document.find_source(&removed).unwrap();
    document.project.remove_from_filmstrip(missing_id).unwrap();
    document.project.remove_from_filmstrip(removed_id).unwrap();
    std::fs::remove_file(&missing).unwrap();

    document
        .redevelop_for_input(crate::processing::InputSettings::Digital)
        .unwrap();
    assert_eq!(
        document
            .project
            .source(missing_id)
            .unwrap()
            .development
            .decode
            .fixed_wb,
        crate::project::UNITY_WB
    );
    assert_eq!(
        document
            .project
            .source(removed_id)
            .unwrap()
            .development
            .decode
            .fixed_wb,
        crate::project::UNITY_WB
    );
    assert_eq!(
        document
            .project
            .source(document.project.order[0])
            .unwrap()
            .development
            .decode
            .fixed_wb,
        read_source(&first, None)
            .unwrap()
            .development
            .decode
            .fixed_wb
    );

    document.import(std::slice::from_ref(&removed)).unwrap();
    assert!(document.project.order.contains(&removed_id));
    assert_eq!(
        document.project.source(removed_id).unwrap().development,
        read_source(&removed, None).unwrap().development
    );
}

#[test]
fn stage_redevelopment_clears_flat_field_correction_and_leaves_the_reference_dormant() {
    let original =
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"))
            .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let photo = directory.path().join("photo.dng");
    let reference = directory.path().join("reference.dng");
    std::fs::write(&photo, &original).unwrap();
    std::fs::write(
        &reference,
        super::super::tests::dng_with_red_as_shot(&original),
    )
    .unwrap();
    let mut document = Document::default();
    document.import(std::slice::from_ref(&photo)).unwrap();
    let reference = read_source(&reference, Some([1.; 4])).unwrap();
    let reference_id = document.register_reference(reference.clone()).unwrap();
    let source = document.project.source(document.project.order[0]).unwrap();
    let map = std::sync::Arc::new(
        crate::flat_field::FlatField::from_nodes(
            source.development.active_area,
            [2, 2],
            vec![glam::DVec3::ONE; 4],
        )
        .unwrap(),
    );
    let field =
        crate::project::FlatFieldSettings::new(document.project.source(reference_id).unwrap(), map);
    document.project.set_flat_field(Some(field));

    document
        .redevelop_for_input(crate::processing::InputSettings::Digital)
        .unwrap();
    assert!(document.project.flat_field().is_none());
    assert_eq!(
        document.project.source(reference_id).unwrap().development,
        reference.development
    );
    let refreshed = read_source(&reference.path, None).unwrap();
    assert_eq!(
        document.register_reference(refreshed.clone()).unwrap(),
        reference_id
    );
    assert_eq!(
        document.project.source(reference_id).unwrap().development,
        refreshed.development
    );

    document
        .redevelop_for_input(crate::processing::InputSettings::Negative {
            dmin: None,
            dmax: None,
            input_floor: crate::processing::CAMERA_INPUT_FLOOR,
            response: crate::processing::Response::film(),
            gamma: crate::processing::DEFAULT_GAMMA,
            balance: Balance::default(),
        })
        .unwrap();
    assert!(document.project.flat_field().is_none());
}

#[test]
fn negative_import_uses_unity_wb_and_digital_open_preserves_saved_gains() {
    let original =
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"))
            .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let raw_path = directory.path().join("red-neutral.dng");
    std::fs::write(
        &raw_path,
        super::super::tests::dng_with_red_as_shot(&original),
    )
    .unwrap();
    let mut document = Document::default();
    document.import(std::slice::from_ref(&raw_path)).unwrap();
    assert_eq!(document.project.order.len(), 1);
    assert_eq!(
        document.project.development().as_ref().unwrap().fixed_wb,
        crate::project::UNITY_WB
    );
    let custom = [2., 1., 1.5, 1.];
    // Digital saves each source's own as-shot decode identity. Construct that accepted
    // record rather than silently reinterpreting the source from its current file header.
    let raw = RawFile::open_with_wb(&raw_path, Some(custom)).unwrap();
    let actual = source_metadata(&raw_path, &raw.metadata, raw.fixed_wb(custom)).unwrap();
    document.project.input = crate::processing::InputSettings::Digital;
    document.project.scan = None;
    document.project.sources[0].development = actual.development;
    assert_eq!(document.project.acquisition_wb(None), None);
    let project_path = directory.path().join("grade.antitone");
    let saved = document.save(&project_path, Save::New).unwrap();
    let mut reopened = Document::open(&project_path).unwrap();
    assert_eq!(reopened.project, saved.project);
    reopened.import(std::slice::from_ref(&raw_path)).unwrap();
    assert_eq!(reopened.project.development().unwrap().fixed_wb, custom);
    let source = reopened.project.order[0];
    let scan = Scan::open(&reopened.source_path(source).unwrap(), Some(custom)).unwrap();
    assert_eq!(scan.fixed_wb, custom);
    reopened.project.development_mut().unwrap().fixed_wb[3] = 0.;
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
        document.project.scan = Some(crate::project::ScanSetup {
            basis: crate::project::ScanBasis::from_development(
                &read_source(file.path(), Some([1.; 4])).unwrap().development,
            ),
            flat_field: None,
        });
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

#[test]
fn mixed_import_fallback_uses_batch_entry_and_restores_each_new_sources_as_shot_wb() {
    let directory = tempfile::tempdir().unwrap();
    let original =
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"))
            .unwrap();
    let first = directory.path().join("first.dng");
    let second = directory.path().join("different-wb.dng");
    let other_camera = directory.path().join("other-camera.dng");
    std::fs::write(&first, &original).unwrap();
    std::fs::write(
        &second,
        super::super::tests::dng_with_red_as_shot(&original),
    )
    .unwrap();
    let mut different_camera = original.clone();
    let locations: Vec<_> = original
        .windows(b"Authored Bayer patches".len())
        .enumerate()
        .filter_map(|(i, bytes)| (bytes == b"Authored Bayer patches").then_some(i))
        .collect();
    assert_eq!(locations.len(), 2, "Model and UniqueCameraModel");
    for i in locations {
        different_camera[i..i + b"Modified Bayer patches".len()]
            .copy_from_slice(b"Modified Bayer patches");
    }
    std::fs::write(&other_camera, different_camera).unwrap();
    let expected_wb = read_source(&second, None)
        .unwrap()
        .development
        .decode
        .fixed_wb;
    assert_ne!(
        expected_wb,
        read_source(&first, None)
            .unwrap()
            .development
            .decode
            .fixed_wb
    );
    assert_ne!(
        read_source(&first, None).unwrap().development.camera_model,
        read_source(&other_camera, None)
            .unwrap()
            .development
            .camera_model
    );

    for paths in [
        vec![first.clone(), second.clone(), other_camera.clone()],
        vec![first.clone(), other_camera.clone(), second.clone()],
    ] {
        let mut document = Document::default();
        document.import(&paths).unwrap();
        assert_eq!(
            document.project.input,
            crate::processing::InputSettings::Digital
        );
        assert_eq!(document.project.order.len(), 3);
        let second_id = document.find_source(&second).unwrap();
        assert_eq!(
            document
                .project
                .source(second_id)
                .unwrap()
                .development
                .decode
                .fixed_wb,
            expected_wb
        );
    }
    // Auto fallback also redevelops a source that already existed before the batch.
    let mut existing = Document::default();
    existing.import(std::slice::from_ref(&first)).unwrap();
    assert!(existing.project.input.is_negative());
    assert_eq!(
        existing.project.sources[0].development.decode.fixed_wb,
        crate::project::UNITY_WB
    );
    existing
        .import(std::slice::from_ref(&other_camera))
        .unwrap();
    assert_eq!(
        existing.project.input,
        crate::processing::InputSettings::Digital
    );
    for source in &existing.project.sources {
        assert_eq!(
            source.development,
            read_source(&source.path, None).unwrap().development,
            "{}",
            source.path.display()
        );
    }

    // Re-adding a dormant source and then encountering an incompatible source in the same
    // batch also redevelops the re-added source for Digital.
    let mut readded = Document::default();
    readded.import(std::slice::from_ref(&first)).unwrap();
    let first_id = readded.project.order[0];
    readded.project.remove_from_filmstrip(first_id).unwrap();
    readded
        .import(&[first.clone(), other_camera.clone()])
        .unwrap();
    assert_eq!(
        readded.project.input,
        crate::processing::InputSettings::Digital
    );
    for source in &readded.project.sources {
        assert_eq!(
            source.development,
            read_source(&source.path, None).unwrap().development,
            "{}",
            source.path.display()
        );
    }

    // A later batch must not reinterpret an already-established multi-source roll.
    let mut established = Document::default();
    established.import(&[first, second]).unwrap();
    assert!(established.project.input.is_negative());
    assert!(established.import(&[other_camera]).is_err());
    assert!(established.project.input.is_negative());
}

#[test]
fn automatic_import_fallback_does_not_discard_partial_or_nondefault_stage_settings() {
    for case in 0..5 {
        let mut document = Document::default();
        assert!(document.auto_digital_available());
        let crate::processing::InputSettings::Negative {
            dmin,
            dmax,
            gamma,
            balance,
            response,
            ..
        } = &mut document.project.input
        else {
            unreachable!()
        };
        match case {
            0 => *dmin = Some(glam::DVec3::splat(0.8)),
            1 => *dmax = Some(glam::DVec3::splat(0.02)),
            2 => *gamma = 0.9,
            3 => balance.warmth = 0.1,
            4 => *response = crate::processing::Response::Original,
            _ => unreachable!(),
        }
        assert!(
            !document.auto_digital_available(),
            "stage edit {case} must require an explicit switch"
        );
    }
}

#[test]
fn fresh_mixed_scan_import_switches_to_digital_only_before_calibration() {
    fn source(
        path: &std::path::Path,
        camera: &str,
        matrix: [[f64; 3]; 3],
    ) -> crate::project::Source {
        std::fs::write(path, b"scan fixture").unwrap();
        crate::project::Source {
            id: crate::project::SourceId(0),
            path: path.into(),
            development: crate::project::SourceDevelopment {
                decode: Development::new("test-backend".into(), [1.; 4]),
                lens_correction: None,
                dng_development: None,
                dimensions: [900, 600],
                active_area: [0, 0, 900, 600],
                camera_model: camera.into(),
                camera_to_ap1: matrix,
                input_floor: crate::processing::CAMERA_INPUT_FLOOR,
            },
            reference_only: false,
            photo: PhotoSettings::default(),
            local_adjustments: None,
            repairs: Default::default(),
            orientation: Default::default(),
            crop: None,
        }
    }

    let directory = tempfile::tempdir().unwrap();
    let mut document = Document::default();
    document
        .project
        .add_source(source(
            &directory.path().join("a.ARW"),
            "camera-a",
            [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        ))
        .unwrap();
    assert!(document.auto_digital_available());
    let incompatible = source(
        &directory.path().join("b.ARW"),
        "camera-b",
        [[0.9, 0., 0.1], [0., 1.1, 0.], [0.05, 0., 0.95]],
    );
    assert!(document.scan_source_incompatible(&incompatible));

    document.project.set_dmin(Some([0.8; 3]));
    document.project.set_dmax(Some([0.02; 3]));
    assert!(!document.auto_digital_available());
}
