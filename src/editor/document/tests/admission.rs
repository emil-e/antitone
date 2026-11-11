use super::*;
use crate::{
    adjustment_layers::{AdjustmentLayer, AdjustmentLayerCoverage},
    geometry::SourcePoint,
    processing::RollSettings,
    project::AdjustmentLayers,
};
use glam::DVec3;

async fn device() -> wgpu::Device {
    let instance = crate::gpu::instance();
    let adapter = instance.request_adapter(&Default::default()).await.unwrap();
    adapter.request_device(&Default::default()).await.unwrap().0
}

fn layers() -> AdjustmentLayers {
    let mut layers = AdjustmentLayers::default();
    for (name, mask, photo) in [
        (
            "Linear",
            AdjustmentLayerCoverage::Linear {
                start: SourcePoint([100., 100.]),
                end: SourcePoint([800., 500.]),
            },
            AdjustmentLayerSettings {
                exposure: 0.25,
                contrast: 1.01,
                ..Default::default()
            },
        ),
        (
            "Radial",
            AdjustmentLayerCoverage::Radial {
                center: SourcePoint([450., 300.]),
                radii: [300., 200.],
                angle: 0.3,
                feather: 0.8,
            },
            AdjustmentLayerSettings {
                exposure: -0.125,
                saturation: 1.02,
                ..Default::default()
            },
        ),
    ] {
        layers
            .add(
                name.into(),
                AdjustmentLayer {
                    mask,
                    photo,
                    enabled: true,
                    invert: false,
                    strength: 0.6,
                },
            )
            .unwrap();
    }
    layers.next_id = 30;
    layers
}

fn unsupported_locals() -> AdjustmentLayers {
    let mut layers = AdjustmentLayers::default();
    layers
        .add(
            "Unrepresentable mask".into(),
            AdjustmentLayer {
                mask: AdjustmentLayerCoverage::Radial {
                    center: SourcePoint([0., 0.]),
                    radii: [f64::MIN_POSITIVE, 1.],
                    angle: 0.,
                    feather: 0.5,
                },
                photo: AdjustmentLayerSettings {
                    exposure: 1.,
                    ..Default::default()
                },
                enabled: true,
                invert: false,
                strength: 1.,
            },
        )
        .unwrap();
    layers
}

fn calibrated(directory: &Path) -> Document {
    let mut document = fixture(directory);
    document.project.roll = RollSettings::default();
    document.project.set_dmin(Some([0.2, 0.15, 0.1]));
    document.project.set_dmax(Some([0.008, 0.005, 0.002]));
    document.project.sources[0].adjustment_layers = Some(layers());
    document
}

#[test]
fn photographic_construction_is_read_only_and_requires_calibration_only_for_positive_work() {
    pollster::block_on(async {
        let device = device().await;
        let directory = tempfile::tempdir().unwrap();
        let document = calibrated(directory.path());
        let before = document.project.clone();
        let id = document.project.order[0];
        for limits in [device.limits(), wgpu::Limits::default()] {
            crate::editor::tests::photograph(&document, id, &limits).unwrap();
            assert_eq!(document.project, before);
        }
        let mut uncalibrated = document.clone();
        uncalibrated.project.set_dmin(None);
        uncalibrated.project.set_dmax(None);
        uncalibrated.project.require_runtime().unwrap();
        assert!(
            crate::editor::tests::photograph(&uncalibrated, id, &device.limits()).is_err(),
            "valid saved layer intent cannot fabricate a calibrated positive"
        );
        uncalibrated.project.sources[0]
            .adjustment_layers
            .as_mut()
            .unwrap()
            .layers
            .clear();
        uncalibrated.project.require_runtime().unwrap();
    });
}

#[test]
fn per_photo_construction_is_scoped_and_inactive_records_still_obey_data_integrity() {
    pollster::block_on(async {
        let device = device().await;
        let directory = tempfile::tempdir().unwrap();
        let mut document = calibrated(directory.path());
        let good = document.project.order[0];
        let bad = document.project.order[1];
        document.project.source_mut(bad).unwrap().adjustment_layers = Some(unsupported_locals());
        document.project.require_runtime().unwrap();
        let before = document.project.clone();
        crate::editor::tests::photograph(&document, good, &device.limits()).unwrap();
        assert!(crate::editor::tests::photograph(&document, bad, &device.limits()).is_err());
        assert_eq!(document.project, before);
        for record in &mut document
            .project
            .source_mut(bad)
            .unwrap()
            .adjustment_layers
            .as_mut()
            .unwrap()
            .layers
        {
            record.adjustment.enabled = false;
        }
        crate::editor::tests::photograph(&document, bad, &device.limits()).unwrap();
        document
            .project
            .source_mut(bad)
            .unwrap()
            .adjustment_layers
            .as_mut()
            .unwrap()
            .layers[0]
            .adjustment
            .strength = f64::NAN;
        assert!(
            document.project.require_runtime().is_err(),
            "inactive records must still obey the persisted control contract"
        );
    });
}

#[test]
fn correction_retains_calibration_and_project_integrity_checks_processing_choices() {
    let directory = tempfile::tempdir().unwrap();
    let mut document = calibrated(directory.path());
    let calibration = document.project.calibration().unwrap().unwrap();
    let source = &document.project.sources[0];
    let field = Arc::new(
        crate::flat_field::FlatField::from_nodes(
            source.development.active_area,
            [2, 2],
            vec![DVec3::new(1.1, 0.9, 1.2); 4],
        )
        .unwrap(),
    );
    let settings = crate::project::FlatFieldSettings::new(source.path.clone(), field);
    document.project.set_flat_field(Some(settings)).unwrap();
    assert_eq!(Some(calibration), document.project.calibration().unwrap());
    document.project.require_runtime().unwrap();
    let before = document.project.clone();
    document.project.development_mut().unwrap().white_balance[0] = 2.;
    assert!(document.project.require_runtime().is_err());
    document.project = before;
    document.project.processing_model = 3;
    assert!(
        document.project.require_runtime().is_err(),
        "metadata integrity must retain strict model/runtime compatibility"
    );
}

#[test]
fn persistence_preserves_valid_photo_grades_even_when_render_admission_rejects_them() {
    pollster::block_on(async {
        let device = device().await;
        let directory = tempfile::tempdir().unwrap();
        let mut document = calibrated(directory.path());
        let reference = document.project.sources[1].clone();
        document
            .project
            .source_mut(reference.id)
            .unwrap()
            .adjustment_layers = Some(layers());
        let map = Arc::new(
            crate::flat_field::FlatField::from_nodes(
                reference.development.active_area,
                [2, 2],
                vec![DVec3::ONE; 4],
            )
            .unwrap(),
        );
        document
            .project
            .set_flat_field(Some(crate::project::FlatFieldSettings::new(
                reference.path.clone(),
                map,
            )))
            .unwrap();
        let ordered = document.project.order[0];
        document.project.remove_from_filmstrip(ordered).unwrap();
        assert_eq!(document.project.order, vec![reference.id]);
        assert_eq!(document.project.sources.len(), 1);
        assert_eq!(
            document.project.flat_field().unwrap().reference_name,
            "two.ARW"
        );
        let path = directory.path().join("reference.json");
        let saved = document.save(&path, Save::New).unwrap();
        assert!(document.dirty());
        assert!(!saved.dirty());
        for name in ["one.ARW", "two.ARW"] {
            std::fs::remove_file(directory.path().join(name)).unwrap();
        }
        let opened = Document::open(&path).unwrap();
        assert_eq!(opened.project, saved.project);
        assert_eq!(
            opened.project.sources[0]
                .adjustment_layers
                .as_ref()
                .unwrap()
                .next_id,
            30
        );
        let mut rejected = saved.clone();
        rejected.project.sources[0].adjustment_layers = Some(unsupported_locals());
        let before = rejected.project.clone();
        let bytes = std::fs::read(&path).unwrap();
        assert!(
            crate::editor::tests::photograph(&rejected, reference.id, &device.limits()).is_err()
        );
        let written = rejected.save(&path, Save::Replace).unwrap();
        assert_eq!(rejected.project, before);
        assert!(rejected.dirty());
        assert_ne!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(Document::open(&path).unwrap().project, written.project);
        let complete = std::fs::read(&path).unwrap();
        rejected.project.sources[0].development.camera_to_ap1 = [[0.; 3]; 3];
        assert!(rejected.save(&path, Save::Replace).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), complete);
        assert_eq!(saved.project, opened.project);
    });
}

#[test]
fn authored_raw_metadata_open_and_duplicate_import_preserve_linear_radial_records_and_ids() {
    let raw = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng");
    let mut document = Document::default();
    document
        .import_with_progress(std::slice::from_ref(&raw), &Default::default(), |_, _| {})
        .unwrap();
    document.project.set_dmin(Some([0.2, 0.15, 0.1]));
    document.project.set_dmax(Some([0.008, 0.005, 0.002]));
    document.project.sources[0].adjustment_layers = Some(layers());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("masks.json");
    let saved = document.save(&path, Save::New).unwrap();
    let mut opened = Document::open(&path).unwrap();
    assert_eq!(opened.project, saved.project);
    opened
        .import_with_progress(&[raw.clone(), raw], &Default::default(), |_, _| {})
        .unwrap();
    assert_eq!(opened.project, saved.project);
    assert!(!opened.dirty());
}
