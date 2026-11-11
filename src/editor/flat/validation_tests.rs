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
        .set_flat_field(Some(FlatFieldSettings::new("reference.dng", map)))
        .unwrap();
}

#[test]
fn correction_resources_ignore_lut_capacity_and_unrepresentable_photographic_constants() {
    let instance = crate::gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: wgpu::Limits {
            max_storage_buffer_binding_size: 1024,
            ..Default::default()
        },
        ..Default::default()
    }))
    .unwrap();
    let mut document = document();
    let asset = Arc::new(
        crate::looks::LookAsset::new(
            "Unrelated LUT".into(),
            crate::looks::LookOrigin::Imported,
            Arc::new(crate::looks::Lut3D::new(5, [0.; 3], [1.; 3], vec![[0.5; 3]; 125]).unwrap()),
        )
        .unwrap(),
    );
    assert!(crate::gpu::lut3d::Lut::require_limits(&device.limits(), asset.lut()).is_err());
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
    document.project.require_runtime().unwrap();
    let before = document.project.clone();
    let prepared = Correction::prepare(&device, &document, None)
        .unwrap()
        .unwrap();
    assert_eq!(
        prepared.settings,
        *document.project.active_flat_field().unwrap()
    );
    assert!(Arc::ptr_eq(
        &prepared,
        &Correction::prepare(&device, &document, Some(&prepared))
            .unwrap()
            .unwrap()
    ));
    document.project.flat_field_mut().unwrap().enabled = false;
    assert!(
        Correction::prepare(&device, &document, Some(&prepared))
            .unwrap()
            .is_none()
    );
    document.project.set_flat_field(None).unwrap();
    assert!(
        Correction::prepare(&device, &document, Some(&prepared))
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
                    editor.document.correction.prepared().is_some(),
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
fn reusing_a_map_checks_new_source_compatibility_before_returning_the_resource() {
    let editor = crate::editor::tests::authored_fixture(&[]);
    let device = editor.document.visit.working_view.device().unwrap();
    let mut document = document();
    let prepared = Correction::prepare(device, &document, None)
        .unwrap()
        .unwrap();
    let mut second = document.project.sources[0].clone();
    second.id = SourceId(0);
    second.path = "second.dng".into();
    let id = document.register(second).unwrap();
    assert!(Arc::ptr_eq(
        &prepared,
        &Correction::prepare(device, &document, Some(&prepared))
            .unwrap()
            .unwrap()
    ));
    document
        .project
        .source_mut(id)
        .unwrap()
        .development
        .camera_to_ap1[0][0] *= 2.;
    document.project.require_runtime().unwrap();
    let before = document.project.clone();
    assert!(
        Correction::prepare(device, &document, Some(&prepared)).is_err(),
        "an unchanged map cannot vouch for a newly incompatible development"
    );
    assert_eq!(document.project, before);
}

#[test]
fn correction_rejects_overflow_of_an_individually_representable_map_and_matrix_atomically() {
    let mut editor = crate::editor::tests::authored_fixture(&[]);
    let device = editor.document.visit.working_view.device().unwrap().clone();
    let mut document = document();
    document.project.sources[0].development.camera_to_ap1 =
        (glam::DMat3::IDENTITY * 1e15).to_cols_array_2d();
    document.project.reset_scan_basis();
    set_map(&mut document, 1.);
    let prepared = Correction::prepare(&device, &document, None)
        .unwrap()
        .unwrap();
    let mut candidate = document.clone();
    set_map(&mut candidate, 1e30);
    candidate.project.require_runtime().unwrap();
    GpuFlatField::new(
        &device,
        candidate.project.active_flat_field().unwrap().map.clone(),
    )
    .unwrap();
    editor.document = document.into();
    editor.document.visit.working_view = super::super::working_view::WorkingView::new(
        device,
        editor.source_services.as_ref().unwrap().queue.clone(),
    );
    editor.document.correction = Some(prepared.clone()).into();
    let before = editor.document.project.clone();
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.start_correction(candidate, cx));
        app.run_until(std::time::Duration::from_secs(15), |editor| {
            editor.document.flat_correction.is_none()
        });
        app.read(|editor| {
            assert_eq!(editor.document.project, before);
            assert!(Arc::ptr_eq(
                editor.document.correction.prepared().unwrap(),
                &prepared
            ));
            assert!(
                editor
                    .history_label(super::super::history::Direction::Undo)
                    .is_none()
            );
            assert_eq!(
                editor.dialogs.message().unwrap().title,
                FailureCase::FlatCorrection.title()
            );
        });
    });
}
