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
        document.project.development().as_ref().unwrap().fixed_wb,
        [1.; 4]
    );
    let custom = [2., 1., 1.5, 1.];
    // A saved SDK WB has a matching matrix/precision/program identity. Construct
    // that accepted basis rather than silently reinterpreting an older source record.
    let raw = RawFile::open_with_wb(&raw_path, Some(custom)).unwrap();
    let actual = source_metadata(&raw_path, &raw.metadata, raw.fixed_wb(custom)).unwrap();
    document.project.scan.as_mut().unwrap().basis = crate::project::ScanBasis::new(&actual);
    document.project.sources[0].development = actual.development;
    assert_eq!(document.project.acquisition_wb(None), Some(custom));
    let directory = tempfile::tempdir().unwrap();
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
            basis: crate::project::ScanBasis {
                development: read_source(file.path(), Some([1.; 4])).unwrap().development,
            },
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
    let mut different_wb = original.clone();
    assert_eq!(&different_wb[..2], b"II");
    let ifd = u32::from_le_bytes(different_wb[4..8].try_into().unwrap()) as usize;
    let count = u16::from_le_bytes(different_wb[ifd..ifd + 2].try_into().unwrap());
    let entry = (0..usize::from(count))
        .map(|i| ifd + 2 + i * 12)
        .find(|&i| u16::from_le_bytes(different_wb[i..i + 2].try_into().unwrap()) == 50728)
        .expect("authored AsShotNeutral");
    assert_eq!(
        u16::from_le_bytes(different_wb[entry + 2..entry + 4].try_into().unwrap()),
        5
    );
    let offset =
        u32::from_le_bytes(different_wb[entry + 8..entry + 12].try_into().unwrap()) as usize;
    // Only the first neutral denominator changes: the declared as-shot red gain is now 2.
    assert_eq!(&different_wb[offset..offset + 8], &[1, 0, 0, 0, 1, 0, 0, 0]);
    different_wb[offset + 4..offset + 8].copy_from_slice(&2_u32.to_le_bytes());
    std::fs::write(&second, different_wb).unwrap();
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
                decode: Development::new("test-backend".into(), [2., 1., 1.5, 1.]),
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
