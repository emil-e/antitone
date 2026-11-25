use super::*;
use crate::application::test_support::with_application;

fn document() -> Document {
    let mut document = Document::default();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/roll-0.dng");
    document.import(&[path]).unwrap();
    document.project.set_dmin(Some([0.8; 3]));
    document.project.set_dmax(Some([0.02; 3]));
    set_map(&mut document, 1.05);
    document
}

fn set_map(document: &mut Document, gain: f64) {
    let area = document.project.sources[0].development.active_area;
    let map = Arc::new(
        crate::flat_field::FlatField::from_nodes(area, [2, 2], vec![DVec3::splat(gain); 4])
            .unwrap(),
    );
    document
        .project
        .set_flat_field(Some(FlatFieldSettings::new("reference.dng", map)));
}

fn limited_device() -> (wgpu::Device, crate::gpu::submission::Queue) {
    let instance = crate::gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    pollster::block_on(crate::gpu::submission::request_device(
        &adapter,
        &wgpu::DeviceDescriptor {
            required_limits: wgpu::Limits {
                max_storage_buffer_binding_size: 1024,
                ..Default::default()
            },
            ..Default::default()
        },
    ))
    .unwrap()
}

#[test]
fn correction_resources_ignore_lut_capacity_and_unrepresentable_photographic_constants() {
    let (device, queue) = limited_device();
    let mut document = document();
    let asset = Arc::new(
        crate::looks::LookAsset::new(
            "Unrelated LUT".into(),
            crate::looks::LookOrigin::Imported,
            Arc::new(crate::looks::Lut3D::new(5, [0.; 3], [1.; 3], vec![[0.5; 3]; 125]).unwrap()),
        )
        .unwrap(),
    );
    assert!(
        crate::photographic::stages::look::Lut::require_limits(&device.limits(), asset.lut())
            .is_err()
    );
    document.project.output.lut = Some(asset);
    let source = &mut document.project.sources[0];
    let mut layers = crate::project::AdjustmentLayers::default();
    for _ in 0..10 {
        layers
            .add(
                "Accepted exposure".into(),
                crate::adjustment_layers::AdjustmentLayer::global(AdjustmentLayerSettings {
                    exposure: 16.,
                    ..Default::default()
                }),
            )
            .unwrap();
    }
    source.adjustment_layers = Some(layers);
    // These valid f64 calibration anchors cannot be represented by the photographic shader ABI.
    let dmin = 1e-50_f64;
    let dmax = 1e-51_f64;
    document.project.set_dmin(Some([dmin; 3]));
    document.project.set_dmax(Some([dmax; 3]));
    assert_eq!(dmin as f32, 0.);
    assert_eq!(dmax as f32, 0.);
    assert!(
        crate::editor::processing::validate_source_with_matrix(
            Some(&device),
            document.project.input,
            document.project.roll,
            &document.project.sources[0],
            glam::DMat3::from_cols_array_2d(&document.project.sources[0].development.camera_to_ap1),
        )
        .is_err(),
        "the captured photograph must really fail shader-constant admission"
    );
    serde_json::to_value(&document.project).unwrap();
    let before = document.project.clone();
    let prepared = Correction::prepare(&device, document.project.active_flat_field(), None)
        .unwrap()
        .unwrap();
    assert_eq!(
        prepared.gpu.field(),
        &document.project.active_flat_field().unwrap().map
    );
    assert!(Arc::ptr_eq(
        &prepared,
        &Correction::prepare(
            &device,
            document.project.active_flat_field(),
            Some(&prepared)
        )
        .unwrap()
        .unwrap()
    ));
    document.project.flat_field_mut().unwrap().enabled = false;
    assert!(
        Correction::prepare(
            &device,
            document.project.active_flat_field(),
            Some(&prepared)
        )
        .unwrap()
        .is_none()
    );
    document.project.set_flat_field(None);
    assert!(
        Correction::prepare(
            &device,
            document.project.active_flat_field(),
            Some(&prepared)
        )
        .unwrap()
        .is_none()
    );
    assert_eq!(document.project.sources, before.sources);
    assert_eq!(document.project.output, before.output);

    document.project = before.clone();
    let mut editor = Editor::new(&device, &queue);
    editor.document = document.into();
    editor.document.visit.working_view =
        super::super::working_view::WorkingView::new(device, queue);
    editor.document.correction = Some(prepared).into();
    with_application(editor, |app| {
        for enabled in [Some(false), Some(true), None] {
            app.enter(|editor, cx| editor.change_correction(enabled, cx));
            app.run_until(std::time::Duration::from_secs(15), |editor| {
                editor.document.flat_correction.is_none()
            });
            app.read(|editor| {
                assert!(!editor.dialogs.is_open(), "{}", editor.status);
                assert_eq!(editor.document.project.sources, before.sources);
                assert_eq!(editor.document.project.output, before.output);
                assert_eq!(
                    editor
                        .document
                        .project
                        .flat_field()
                        .map(|field| field.enabled),
                    enabled
                );
                assert_eq!(
                    editor.correction().prepared().is_some(),
                    enabled == Some(true)
                );
                assert_eq!(
                    editor.history_label(super::super::history::Direction::Undo),
                    Some("Correction")
                );
            });
        }
    });
}

#[test]
fn map_upload_failure_preserves_the_previous_correction_and_history() {
    let (device, queue) = limited_device();
    let document = document();
    let prepared = Correction::prepare(&device, document.project.active_flat_field(), None)
        .unwrap()
        .unwrap();
    let mut candidate = document.clone();
    let area = candidate.project.sources[0].development.active_area;
    let map = Arc::new(
        crate::flat_field::FlatField::from_nodes(area, [11, 11], vec![DVec3::ONE; 121]).unwrap(),
    );
    assert!(map.nodes().len() as u64 * 12 > device.limits().max_storage_buffer_binding_size);
    candidate
        .project
        .set_flat_field(Some(FlatFieldSettings::new("larger.dng", map)));
    serde_json::to_value(&candidate.project).unwrap();
    let before = document.project.clone();
    let mut editor = Editor::new(&device, &queue);
    editor.document = document.into();
    editor.document.visit.working_view =
        super::super::working_view::WorkingView::new(device, queue);
    editor.document.correction = Some(prepared.clone()).into();
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.start_correction(candidate, cx));
        app.run_until(std::time::Duration::from_secs(15), |editor| {
            editor.document.flat_correction.is_none()
        });
        app.read(|editor| {
            assert_eq!(editor.document.project, before);
            assert!(Arc::ptr_eq(
                editor.correction().prepared().unwrap(),
                &prepared
            ));
            for direction in [
                super::super::history::Direction::Undo,
                super::super::history::Direction::Redo,
            ] {
                assert!(editor.history_label(direction).is_none());
            }
            assert_eq!(
                editor.dialogs.message().unwrap().title,
                FailureCase::FlatCorrection.title()
            );
        });
    });
}

#[test]
fn preparing_and_reusing_a_map_are_independent_of_photo_camera_descriptors() {
    let editor = crate::editor::tests::authored_fixture(&[]);
    let device = editor.document.visit.working_view.device().unwrap();
    let mut document = document();
    let prepared = Correction::prepare(device, document.project.active_flat_field(), None)
        .unwrap()
        .unwrap();
    let mut second = document.project.sources[0].clone();
    second.id = SourceId(0);
    second.path = "second.dng".into();
    let id = document.register(second).unwrap();
    assert!(Arc::ptr_eq(
        &prepared,
        &Correction::prepare(
            device,
            document.project.active_flat_field(),
            Some(&prepared)
        )
        .unwrap()
        .unwrap()
    ));
    document
        .project
        .source_mut(id)
        .unwrap()
        .development
        .camera_to_ap1[0][0] *= 2.;
    document
        .project
        .source_mut(id)
        .unwrap()
        .development
        .camera_model = "Another camera".into();
    serde_json::to_value(&document.project).unwrap();
    let before = document.project.clone();
    for previous in [None, Some(&prepared)] {
        let correction =
            Correction::prepare(device, document.project.active_flat_field(), previous)
                .expect("map preparation does not validate unrelated photographs")
                .unwrap();
        assert_eq!(correction.gpu.field(), prepared.gpu.field());
        if let Some(previous) = previous {
            assert!(Arc::ptr_eq(&correction, previous));
        }
    }
    assert_eq!(document.project, before);
}

#[test]
fn correction_acceptance_is_independent_of_photographic_overflow_which_fails_at_execution() {
    let mut editor = crate::editor::tests::authored_fixture(&["roll-0.dng"]);
    let codes =
        crate::gpu::source::camera_source(&editor.source_context().unwrap().scan().unwrap())
            .unwrap();
    let device = editor.document.visit.working_view.device().unwrap().clone();
    let mut document = document();
    document.project.sources[0].development.camera_to_ap1 =
        (glam::DMat3::IDENTITY * 1e15).to_cols_array_2d();

    set_map(&mut document, 1.);
    let prepared = Correction::prepare(&device, document.project.active_flat_field(), None)
        .unwrap()
        .unwrap();
    let mut candidate = document.clone();
    set_map(&mut candidate, 1e30);
    serde_json::to_value(&candidate.project).unwrap();
    let large_map = Arc::new(
        GpuFlatField::new(
            &device,
            candidate.project.active_flat_field().unwrap().map.clone(),
        )
        .unwrap(),
    );
    let id = document.project.order[0];
    let recipe = document.image_recipe(id).unwrap();
    let small = crate::photographic::graph::composition::original_camera(
        codes.clone(),
        Some(prepared.gpu.clone()),
        &recipe,
    );
    assert!(small.is_ok(), "the large matrix alone is executable");
    let mut recipe = candidate.image_recipe(id).unwrap();
    let overflow = crate::photographic::graph::composition::original_camera(
        codes.clone(),
        Some(large_map.clone()),
        &recipe,
    );
    assert!(
        matches!(overflow, Err(Error::Gpu(_))),
        "photographic execution rejects the overflowing combination"
    );
    let mut ordinary = document.clone();
    ordinary.project.sources[0].development.camera_to_ap1 =
        glam::DMat3::IDENTITY.to_cols_array_2d();

    recipe.neutral.source_development = ordinary
        .image_recipe(id)
        .unwrap()
        .neutral
        .source_development;
    assert!(
        crate::photographic::graph::composition::original_camera(codes, Some(large_map), &recipe)
            .is_ok(),
        "the large map alone is executable"
    );
    let expected = candidate.project.clone();
    let before = document.project.clone();
    editor.document = document.into();
    editor.document.visit.working_view = super::super::working_view::WorkingView::new(
        device,
        editor.source_services.as_ref().unwrap().queue.clone(),
    );
    editor.document.correction = Some(prepared.clone()).into();
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.start_correction(candidate, cx));
        app.run_until(std::time::Duration::from_secs(15), |editor| {
            editor.document.flat_correction.is_none()
        });
        app.read(|editor| {
            assert_eq!(editor.document.project, expected);
            assert_eq!(
                editor.correction().prepared().unwrap().gpu.field(),
                &expected.active_flat_field().unwrap().map
            );
            assert!(!editor.dialogs.is_open(), "{}", editor.status);
            assert_eq!(
                editor.history_label(super::super::history::Direction::Undo),
                Some("Correction")
            );
        });
        app.enter(|editor, cx| {
            editor.restore_history(super::super::history::Direction::Undo, cx);
            assert_eq!(editor.document.project, before);
            assert!(
                editor
                    .history_label(super::super::history::Direction::Undo)
                    .is_none()
            );
            assert_eq!(
                editor.history_label(super::super::history::Direction::Redo),
                Some("Correction")
            );
            editor.restore_history(super::super::history::Direction::Redo, cx);
            assert_eq!(editor.document.project, expected);
            assert!(
                editor
                    .history_label(super::super::history::Direction::Redo)
                    .is_none()
            );
        });
    });
}
