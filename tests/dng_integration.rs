#![cfg(all(feature = "raw", feature = "dng-sdk", feature = "project"))]

use antitone::{
    dng::{Cancellation, Header, Options},
    processing::{CAMERA_INPUT_FLOOR, InputSettings},
    project::{Project, Save, Source, SourceDevelopment, SourceId},
    raw::{RawFile, Rect},
    scan::Scan,
    source_store::{SourceKey, SourceStore},
};
use glam::DVec3;
use std::{path::Path, sync::Arc};

#[path = "support/dng_fixture.rs"]
mod fixture;
#[path = "support/fixture_path.rs"]
mod fixture_path;
use fixture::{Fixture, Opcode};

#[cfg(all(
    feature = "gpu",
    feature = "export",
    feature = "ocio-reference",
    target_os = "macos"
))]
#[path = "dng_integration/exports.rs"]
mod exports;

fn record(path: &Path, scan: &Scan) -> Source {
    let metadata = scan.source().metadata();
    let area = scan.area;
    Source {
        id: SourceId(0),
        path: path.into(),
        development: SourceDevelopment {
            decode: metadata.development(scan.source().fixed_wb()),
            lens_correction: metadata.lens_identity().unwrap(),
            dng_development: metadata.dng_identity(),
            dimensions: metadata.source_dimensions().unwrap(),
            active_area: [area.x, area.y, area.width, area.height],
            camera_model: scan.camera_model.clone(),
            camera_to_ap1: scan.camera_to_ap1.to_cols_array_2d(),
            input_floor: scan.source().input_floor(),
        },
        reference_only: false,
        photo: Default::default(),
        repairs: Default::default(),
        local_adjustments: None,
        orientation: Default::default(),
        crop: None,
    }
}

#[test]
fn sdk_scan_uses_original_quantum_fixed_basis_continuous_samples_and_capture_dependencies() {
    let mut fixture = Fixture {
        marker: Some((30, 28, 4160)),
        ..Default::default()
    };
    fixture.lists[1].push(Opcode::gain(0.5, 1));
    fixture.lists[2].push(Opcode::warp(&[[1., 0.02, 0., 0., 0., 0.]], [0.5, 0.5]));
    let file = fixture.write();
    let wb = [2., 1., 1.5, 1.];
    let header = Header::read(
        file.path(),
        Options {
            max_pixels: 4096,
            fixed_wb: Some([2., 1., 1.5]),
        },
        &Cancellation::default(),
    )
    .unwrap();
    let scan = Scan::open(file.path(), Some(wb)).unwrap();
    assert_eq!(scan.fixed_wb, wb);
    assert_eq!(scan.area, Rect::from([3, 5, 48, 46]));
    assert_eq!(scan.source().input_floor(), 4. / 4096.);
    assert_eq!(scan.input_floor, CAMERA_INPUT_FLOOR);
    assert_eq!(
        scan.source().metadata().input_floor().unwrap(),
        header.input_floor
    );
    assert!(scan.source().metadata().lens_identity().unwrap().is_none());
    assert!(scan.source().metadata().dng_identity().is_some());
    let image = scan.source().camera_image().unwrap();
    assert!(image.is_continuous());
    let actual = image.pixel(12, 12).unwrap();
    let expected = DVec3::new(0.25, 0.1875, 0.375);
    assert!((actual - expected).abs().max_element() < 1e-7);
    assert!(
        scan.source()
            .capture_eligibility(Rect::from([24, 24, 1, 1]))
            .unwrap()
            .at_white
    );
    assert!(
        scan.source()
            .capture_eligibility(Rect::from([0, 0, 1, 1]))
            .unwrap()
            .repeated_border
    );
    assert!(
        scan.source()
            .capture_eligibility(Rect::from([12, 12, 1, 1]))
            .unwrap()
            .eligible()
    );
    let neutral = scan.camera_to_ap1 * DVec3::ONE;
    assert!((neutral - DVec3::ONE).abs().max_element() < 2e-6);
    let before = scan
        .source()
        .region_sensor_diagnostics(Rect::from([24, 24, 1, 1]))
        .unwrap();
    assert_eq!(before.at_white[0], 1);
    assert_eq!(before.peak[0], 4160);
    let lease = scan.source().clone();
    drop(scan);
    std::thread::spawn(move || {
        assert_eq!(
            lease
                .region_sensor_diagnostics(Rect::from([24, 24, 1, 1]))
                .unwrap(),
            before
        );
        assert!(lease.camera_image().unwrap().pixel(12, 12).is_ok());
    })
    .join()
    .unwrap();
}

#[test]
fn lossless_jpeg_14bit_preserves_original_codes_and_shared_sdk_precision() {
    let plain = Fixture {
        bits: 14,
        ramp: true,
        marker: Some((30, 28, 65)),
        ..Default::default()
    }
    .write();
    let compressed = Fixture {
        bits: 14,
        lossless_jpeg: true,
        ramp: true,
        marker: Some((30, 28, 65)),
        ..Default::default()
    }
    .write();
    let a = Scan::open(plain.path(), None).unwrap();
    let b = Scan::open(compressed.path(), None).unwrap();
    assert_eq!(
        a.source().sensor_diagnostics().unwrap(),
        b.source().sensor_diagnostics().unwrap()
    );
    assert_eq!(b.source().input_floor(), 1. / 4096.);
    assert_eq!(a.dimensions().unwrap(), b.dimensions().unwrap());
    let (width, height) = a.dimensions().unwrap();
    for y in 0..height {
        for x in 0..width {
            assert_eq!(a.pixel(x, y).unwrap(), b.pixel(x, y).unwrap());
        }
    }
}

#[test]
fn shared_sdk_rejects_in_range_bad_pixel_markers_without_a_libraw_fallback() {
    let mut fixture = Fixture {
        marker: Some((30, 28, 1000)),
        ..Default::default()
    };
    fixture.lists[0].push(Opcode::bad_pixels(1000, 2));
    let file = fixture.write();
    assert!(RawFile::open_uncorrected(file.path()).is_ok());
    assert!(RawFile::open(file.path()).is_err());
    assert!(Scan::open(file.path(), None).is_err());
}

#[test]
fn shared_sdk_bad_pixel_repair_keeps_original_black_capture_rejection() {
    let mut fixture = Fixture {
        marker: Some((30, 28, 64)),
        ..Default::default()
    };
    fixture.lists[0].push(Opcode::bad_pixels(64, 2));
    let file = fixture.write();
    let scan = Scan::open(file.path(), None).unwrap();
    let repaired = scan.pixel(24, 24).unwrap();
    assert!(
        (repaired - DVec3::new(0.25, 0.375, 0.5))
            .abs()
            .max_element()
            < 1e-7
    );
    let state = scan
        .source()
        .capture_eligibility(Rect::from([24, 24, 1, 1]))
        .unwrap();
    assert!(state.below_black && !state.eligible());
    let original = scan
        .source()
        .region_sensor_diagnostics(Rect::from([24, 24, 1, 1]))
        .unwrap();
    assert_eq!(original.below_black[0], 1);
    assert_eq!(scan.source().input_floor(), 2. / 4096.);
}

#[test]
fn sdk_project_roundtrip_and_cache_keep_backend_precision_and_fixed_wb_identity() {
    let file = Fixture::default().write();
    let wb = [2., 1., 1.5, 1.];
    let scan = Scan::open(file.path(), Some(wb)).unwrap();
    let source = record(file.path(), &scan);
    let development = scan.source().metadata().development(wb);
    let mut project = Project {
        input: InputSettings::Digital,
        ..Project::default()
    };
    let _ = CAMERA_INPUT_FLOOR;
    let id = project.add_source(source).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("sdk.antitone");
    project.save(&path, Save::New).unwrap();
    let loaded = Project::load(&path).unwrap();
    loaded.require_runtime(&antitone::raw::version()).unwrap();
    let reopened = Scan::open(
        &loaded.source(id).unwrap().path,
        Some(loaded.sources[0].development.decode.fixed_wb),
    )
    .unwrap();
    assert_eq!(
        record(file.path(), &reopened).development,
        loaded.source(id).unwrap().development
    );
    let key = SourceKey::capture(file.path(), &loaded.source(id).unwrap().development).unwrap();
    let store = SourceStore::new(u64::MAX);
    store
        .publish_source(key.clone(), scan.source().clone())
        .unwrap()
        .unwrap();
    assert!(Arc::ptr_eq(&store.get(&key).unwrap(), scan.source()));
    let mut wrong = development.clone();
    wrong.backend_version.push_str("-different");
    let stale = SourceKey::capture(
        file.path(),
        &antitone::project::SourceDevelopment {
            decode: wrong,
            ..loaded.source(id).unwrap().development.clone()
        },
    )
    .unwrap();
    assert!(store.get(&stale).is_none());
    assert!(store.publish_source(stale, scan.source().clone()).is_err());
    let legacy = antitone::project::Development::new(antitone::raw::version(), wb);
    assert_ne!(
        legacy.contract, loaded.sources[0].development.decode.contract,
        "the DNG source keeps its own decode identity"
    );
}

#[test]
fn sdk_flat_reference_requires_the_same_opcode_interpretation_not_just_geometry_and_floor() {
    let plain = Fixture::default().write();
    let mut fixture = Fixture::default();
    fixture.lists[1].push(Opcode::gain(0.5, 1));
    let gained = fixture.write();
    let a = Scan::open(plain.path(), None).unwrap();
    let b = Scan::open(gained.path(), None).unwrap();
    let reference = record(plain.path(), &a);
    let other = record(gained.path(), &b);
    assert_eq!(
        reference.development.dimensions,
        other.development.dimensions
    );
    assert_eq!(
        reference.development.camera_to_ap1,
        other.development.camera_to_ap1
    );
    assert_eq!(
        reference.development.input_floor,
        other.development.input_floor
    );
    let area = a.area;
    let map = antitone::flat_field::FlatField::from_nodes(
        [area.x, area.y, area.width, area.height],
        [2, 2],
        vec![DVec3::ONE; 4],
    )
    .unwrap();
    let settings = antitone::project::FlatFieldSettings::new(&reference, Arc::new(map));
    settings.require_source(&reference.development).unwrap();
    assert!(settings.require_source(&other.development).is_err());
}

#[test]
fn sdk_project_rejects_unequal_green_or_noncanonical_saved_wb_without_reinterpretation() {
    let file = Fixture::default().write();
    let scan = Scan::open(file.path(), None).unwrap();
    let mut project = Project::default();
    project.add_source(record(file.path(), &scan)).unwrap();
    for wb in [[2., 1., 1.5, 0.75], [4., 2., 3., 2.], [512., 1., 1., 1.]] {
        project.sources[0].development.decode.fixed_wb = wb;
        assert!(project.validate().is_err());
    }
    assert!(Scan::open(file.path(), Some([2., 1., 1.5, 0.75])).is_err());
}

#[test]
fn shared_source_open_rejects_required_instructions_before_any_libraw_fallback() {
    let mut fixture = Fixture::default();
    fixture.lists[1].push(Opcode::unknown(false));
    let file = fixture.write();
    assert!(RawFile::open(file.path()).is_err());
    assert!(Scan::open(file.path(), None).is_err());
}

#[test]
fn header_open_does_not_accept_missing_sensor_payload_as_a_completed_source() {
    let file = Fixture::default().write();
    let mut bytes = std::fs::read(file.path()).unwrap();
    bytes.truncate(bytes.len() - 64 * 64 * 2);
    std::fs::write(file.path(), bytes).unwrap();
    assert!(RawFile::open(file.path()).is_ok());
    assert!(Scan::open(file.path(), None).is_err());
}

#[test]
#[ignore = "read-only supplied cameras; set ANTITONE_DNG_SAMPLE_DIR, full-resolution cold development"]
fn supplied_cameras_share_editor_source_and_saved_project_contracts() {
    let directory =
        std::env::var_os("ANTITONE_DNG_SAMPLE_DIR").expect("set ANTITONE_DNG_SAMPLE_DIR");
    for name in [
        "leica-m9.dng",
        "ricoh-gr-iii.dng",
        "ricoh-gr-iiix.dng",
        "leica-q2.dng",
    ] {
        let path = Path::new(&directory).join(name);
        let started = std::time::Instant::now();
        let scan = Scan::open(&path, None).unwrap();
        let wb = scan.fixed_wb;
        let dimensions = scan.dimensions().unwrap();
        let area = scan.area;
        let point = (area.x + area.width / 2, area.y + area.height / 2);
        let before = scan
            .source()
            .camera_image()
            .unwrap()
            .pixel(point.0, point.1)
            .unwrap();
        assert!(before.is_finite());
        assert!(scan.source().camera_image().unwrap().is_continuous());
        assert!(scan.source().metadata().dng_identity().is_some());
        let (w, h, proxy) = scan.preview_source(256).unwrap();
        assert_eq!(proxy.len(), w as usize * h as usize);
        let mut project = Project {
            input: InputSettings::Digital,
            ..Project::default()
        };
        let id = project.add_source(record(&path, &scan)).unwrap();
        let temporary = tempfile::tempdir().unwrap();
        let destination = temporary.path().join("camera.antitone");
        project.save(&destination, Save::New).unwrap();
        println!(
            "{name} dimensions={dimensions:?} area={area:?} WB={wb:?} floor={} retained_bytes={:?} cold_seconds={:.3}",
            scan.source().input_floor(),
            scan.source().retained_bytes(),
            started.elapsed().as_secs_f64()
        );
        drop(scan);
        let loaded = Project::load(&destination).unwrap();
        loaded.require_runtime(&antitone::raw::version()).unwrap();
        let source = loaded.source(id).unwrap();
        let reopened = Scan::open(
            &source.path,
            Some(loaded.sources[0].development.decode.fixed_wb),
        )
        .unwrap();
        assert_eq!(record(&path, &reopened).development, source.development);
        assert_eq!(
            reopened
                .source()
                .camera_image()
                .unwrap()
                .pixel(point.0, point.1)
                .unwrap(),
            before
        );
        let key = SourceKey::capture(&source.path, &source.development).unwrap();
        let store = SourceStore::new(u64::MAX);
        store
            .publish_source(key.clone(), reopened.source().clone())
            .unwrap()
            .unwrap();
        assert!(Arc::ptr_eq(&store.get(&key).unwrap(), reopened.source()));
    }
}

#[test]
#[ignore = "read-only supplied M9; full-resolution decode, bounded proxy/measurement outputs"]
fn supplied_m9_shared_source_and_project_roundtrip() {
    let path = std::env::var_os("ANTITONE_M9_SAMPLE").expect("set ANTITONE_M9_SAMPLE");
    let path = Path::new(&path);
    let scan = Scan::open(path, None).unwrap();
    let dimensions = scan.dimensions().unwrap();
    assert_eq!(dimensions, (5216, 3472));
    assert_eq!(scan.area, Rect::from([2, 2, 5212, 3468]));
    let wb = scan.fixed_wb;
    let before = scan
        .source()
        .camera_image()
        .unwrap()
        .pixel(1024, 1024)
        .unwrap();
    let (w, h, proxy) = scan.preview_source(256).unwrap();
    assert_eq!(proxy.len(), w as usize * h as usize);
    let mut project = Project {
        input: InputSettings::Digital,
        ..Project::default()
    };
    let id = project.add_source(record(path, &scan)).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("m9.antitone");
    project.save(&destination, Save::New).unwrap();
    drop(scan);
    let loaded = Project::load(&destination).unwrap();
    loaded.require_runtime(&antitone::raw::version()).unwrap();
    let source = loaded.source(id).unwrap();
    let scan = Scan::open(
        &source.path,
        Some(loaded.sources[0].development.decode.fixed_wb),
    )
    .unwrap();
    assert_eq!(record(path, &scan).development, source.development);
    assert_eq!(
        scan.source()
            .camera_image()
            .unwrap()
            .pixel(1024, 1024)
            .unwrap(),
        before
    );
    println!(
        "M9 shared/project dimensions={dimensions:?} WB={wb:?} floor={} retained_bytes={:?} proxy={w}x{h}",
        scan.source().input_floor(),
        scan.source().retained_bytes()
    );
}
