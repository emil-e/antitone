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
    let base = self::layers().layers[0].adjustment;
    for exposure in [-16., 16.] {
        for _ in 0..7 {
            layers
                .add(
                    "Recovery".into(),
                    AdjustmentLayer {
                        photo: AdjustmentLayerSettings {
                            exposure,
                            ..Default::default()
                        },
                        strength: 1.,
                        ..base
                    },
                )
                .unwrap();
        }
    }
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
fn complete_stack_preflight_is_read_only_with_adapter_or_default_limits() {
    pollster::block_on(async {
        let device = device().await;
        let directory = tempfile::tempdir().unwrap();
        let document = calibrated(directory.path());
        let before = document.project.clone();
        document.validate_for_device(&device).unwrap();
        document.validate_for_device(&device).unwrap();
        assert_eq!(document.project, before);

        document.validate().unwrap();
        assert_eq!(document.project, before);
        let mut uncalibrated = document.clone();
        uncalibrated.project.set_dmin(None);
        uncalibrated.project.set_dmax(None);
        uncalibrated.validate_for_device(&device).unwrap();
        uncalibrated.validate().unwrap();
        assert!(
            crate::gpu::GpuPipeline::new(
                &device,
                uncalibrated.project.input,
                uncalibrated.project.roll,
                uncalibrated.project.sources[0].grade_snapshot().unwrap(),
                glam::DMat3::IDENTITY,
                uncalibrated.project.sources[0]
                    .development
                    .active_area
                    .into()
            )
            .is_err(),
            "saved layer intent remains valid while positive execution waits for interpretation"
        );
        uncalibrated.project.sources[0]
            .adjustment_layers
            .as_mut()
            .unwrap()
            .layers
            .clear();
        uncalibrated.validate_for_device(&device).unwrap();
    });
}

#[test]
fn device_preflight_includes_offscreen_photos_and_the_flat_field_reference_and_validates_inactive_records()
 {
    pollster::block_on(async {
        let device = device().await;
        let directory = tempfile::tempdir().unwrap();
        for reference_only in [false, true] {
            let mut document = calibrated(directory.path());
            let id = document.project.sources[1].id;
            if reference_only {
                document.project.order.retain(|source| *source != id);
                let source = document.project.source(id).unwrap();
                let map = Arc::new(
                    crate::flat_field::FlatField::from_nodes(
                        source.development.active_area,
                        [2, 2],
                        vec![DVec3::ONE; 4],
                    )
                    .unwrap(),
                );
                let field = crate::project::FlatFieldSettings::new(source, map);
                document.project.set_flat_field(Some(field));
                assert!(document.project.source(id).unwrap().reference_only);
            }
            document.project.source_mut(id).unwrap().adjustment_layers = Some(unsupported_locals());
            let before = document.project.clone();
            assert!(document.validate_for_device(&device).is_err());
            assert_eq!(document.project, before);
            let layers = document
                .project
                .source_mut(id)
                .unwrap()
                .adjustment_layers
                .as_mut()
                .unwrap();
            for record in &mut layers.layers {
                record.adjustment.enabled = false;
            }
            document.validate_for_device(&device).unwrap();
            document
                .project
                .source_mut(id)
                .unwrap()
                .adjustment_layers
                .as_mut()
                .unwrap()
                .layers[0]
                .adjustment
                .strength = f64::NAN;
            assert!(
                document.validate_for_device(&device).is_err(),
                "inactive records must still obey the persisted mask/control contract"
            );
        }
    });
}

#[test]
fn document_device_preflight_uses_the_effective_correction_floor_and_runtime_basis() {
    pollster::block_on(async {
        let device = device().await;
        let directory = tempfile::tempdir().unwrap();
        let mut document = calibrated(directory.path());
        let source = &document.project.sources[0];
        let field = Arc::new(
            crate::flat_field::FlatField::from_nodes(
                source.development.active_area,
                [2, 2],
                vec![DVec3::new(1.1, 0.9, 1.2); 4],
            )
            .unwrap(),
        );
        document
            .project
            .set_flat_field(Some(crate::project::FlatFieldSettings::new(
                source,
                field.clone(),
            )));
        let calibration = document.project.processing_calibration().unwrap().unwrap();
        assert_eq!(calibration.input_floor, field.precision_floor().unwrap());
        document.validate_for_device(&device).unwrap();
        let before = document.project.clone();
        document.project.development_mut().unwrap().fixed_wb[1] = 0.;
        assert!(document.validate_for_device(&device).is_err());
        document.project = before;
        document.project.processing_model = 3;
        assert!(
            document.validate_for_device(&device).is_err(),
            "device preflight cannot bypass strict model/runtime compatibility"
        );
    });
}

#[test]
fn device_aware_save_and_open_validate_flat_field_reference_grades_before_io_and_preserve_rejected_candidates()
 {
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
                &reference, map,
            )));
        let ordered = document.project.order[0];
        document.project.remove_from_filmstrip(ordered).unwrap();
        document
            .project
            .remove_from_filmstrip(reference.id)
            .unwrap();
        assert!(document.project.order.is_empty());
        assert_eq!(document.project.sources.len(), 1);
        assert!(
            document
                .project
                .source(reference.id)
                .unwrap()
                .reference_only
        );
        let path = directory.path().join("reference.json");
        let saved = document
            .save_with_device(&path, Save::New, Some(&device))
            .unwrap();
        assert!(document.dirty());
        assert!(!saved.dirty());
        for name in ["one.ARW", "two.ARW"] {
            std::fs::remove_file(directory.path().join(name)).unwrap();
        }
        let opened = Document::open_with_progress(&path, Some(&device), |_, _| {}).unwrap();
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
            rejected
                .save_with_device(&path, Save::Replace, Some(&device))
                .is_err()
        );
        assert_eq!(rejected.project, before);
        assert!(rejected.dirty());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        rejected.project.save(&path, Save::Replace).unwrap();
        let mut progress = Vec::new();
        assert!(
            Document::open_with_progress(&path, Some(&device), |done, total| progress
                .push((done, total)))
            .is_err()
        );
        assert!(
            progress.is_empty(),
            "reject unsupported grades before reading source files"
        );
        assert_eq!(saved.project, opened.project);
    });
}

#[test]
fn unsupported_offscreen_grade_blocks_source_and_export_before_retiring_accepted_state_or_dispatch()
{
    pollster::block_on(async {
        let (mut editor, _, _) = crate::editor::processing::tests::fixture().await;
        editor.sync_document();
        editor
            .validate_document_candidate(&editor.document)
            .unwrap();
        let accepted = editor
            .positive_processing(editor.input, editor.roll, editor.adjustment_layer_settings)
            .unwrap();
        let mut retired = editor
            .document
            .project
            .source(editor.current_source.unwrap())
            .unwrap()
            .clone();
        retired.id = SourceId(0);
        retired.path = "retired.ARW".into();
        retired.adjustment_layers = Some(unsupported_locals());
        let id = editor.document.project.add_source(retired).unwrap();
        assert!(editor.document.project.order.contains(&id));
        let before = editor.document.project.clone();
        let selected = editor.current_source;
        let loaded = editor.id;
        let epoch = editor
            .source_epoch
            .load(std::sync::atomic::Ordering::Relaxed);
        let revision = editor.original.revision();
        let viewport = editor.viewport;
        let path = editor.document.visit.ready.as_ref().unwrap().path.clone();
        editor.load(
            "different.ARW".into(),
            &crate::application::UiContext::render_only(),
        );
        assert!(
            editor.status.contains("f32 intermediate working range"),
            "{}",
            editor.status
        );
        assert_eq!(editor.current_source, selected);
        assert_eq!(editor.id, loaded);
        assert_eq!(
            editor
                .source_epoch
                .load(std::sync::atomic::Ordering::Relaxed),
            epoch
        );
        assert_eq!(editor.document.visit.ready.as_ref().unwrap().path, path);
        assert_eq!(editor.viewport, viewport);
        assert_eq!(editor.original.revision(), revision);
        assert_eq!(editor.document.project, before);
        assert!(
            !editor.document.visit.source.pending() && editor.export_operation.pending.is_none()
        );
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("existing.tif");
        std::fs::write(&destination, b"keep destination").unwrap();
        editor.export(
            destination.clone(),
            crate::editor::OutputFormat::Tiff(crate::rendering::OutputTarget::Srgb),
            crate::export::Overwrite::Refuse,
            &crate::application::UiContext::render_only(),
        );
        assert!(
            editor.status.contains("f32 intermediate working range"),
            "{}",
            editor.status
        );
        assert!(!editor.busy);
        assert!(editor.export_operation.pending.is_none());
        assert!(
            !editor.document.visit.source.pending() && editor.export_operation.pending.is_none()
        );
        assert_eq!(std::fs::read(&destination).unwrap(), b"keep destination");
        assert_eq!(editor.document.project, before);
        let current = editor
            .positive_processing(editor.input, editor.roll, editor.adjustment_layer_settings)
            .unwrap();
        assert!(std::ptr::eq(
            accepted.pipeline().unwrap(),
            current.pipeline().unwrap()
        ));
    });
}

#[test]
fn device_aware_authored_raw_open_and_duplicate_import_preserve_linear_radial_records_and_ids() {
    pollster::block_on(async {
        let device = device().await;
        let raw = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng");
        let mut document = Document::default();
        document
            .import_with_progress(std::slice::from_ref(&raw), Some(&device), |_, _| {})
            .unwrap();
        document.project.set_dmin(Some([0.2, 0.15, 0.1]));
        document.project.set_dmax(Some([0.008, 0.005, 0.002]));
        document.project.sources[0].adjustment_layers = Some(layers());
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("masks.json");
        let saved = document
            .save_with_device(&path, Save::New, Some(&device))
            .unwrap();
        let mut opened = Document::open_with_progress(&path, Some(&device), |_, _| {}).unwrap();
        assert_eq!(opened.project, saved.project);
        opened
            .import_with_progress(&[raw.clone(), raw], Some(&device), |_, _| {})
            .unwrap();
        assert_eq!(opened.project, saved.project);
        assert!(!opened.dirty());
    });
}
