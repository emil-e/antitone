use super::*;
use crate::{exposure_reference::ReferenceExposure, processing::InputSettings};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use crate::exposure_fixture as preview_fixture;

#[test]
fn digital_acquisition_resolves_before_ready_reuses_native_work_and_leaves_other_photos_lazy() {
    let first = preview_fixture::neutral_dng(1.3);
    let second = preview_fixture::neutral_dng(0.6);
    let mut document = Document::default();
    document.project.input = InputSettings::Digital;
    document
        .import(&[first.path().to_owned(), second.path().to_owned()])
        .unwrap();
    document.cache().set_budget(64 << 20);
    let before = document.project.clone();
    let first_id = document.project.order[0];
    let first_record = document.project.source(first_id).unwrap().clone();
    let requests = Arc::new(AtomicUsize::new(0));
    let observed = requests.clone();
    let scheduler = development::Scheduler::new_observed(move |_, phase, _| {
        if matches!(phase, development::NativePhase::UnpackBegin) {
            observed.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    });
    let request = |document: &Document| Request {
        input: document.project.input,
        path: first.path().to_owned(),
        wb: Some(first_record.development.decode.fixed_wb),
        expected: Some(document.project.source(first_id).unwrap().clone()),
        cache: Some(Cache::new(first.path(), Some(&first_record), document)),
        correction: None,
    };
    let developed = request(&document).load(&scheduler, &|| false).unwrap();
    let source = developed.context.source;
    let reference = source
        .reference_exposure
        .get()
        .expect("acquisition must resolve before publication")
        .clone();
    assert!(
        matches!(reference, ReferenceExposure::Matched(_)),
        "{reference:?}"
    );
    assert!((reference.ev() - 1.3).abs() < 0.03, "{reference:?}");
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    assert_eq!(document.project, before);
    let second_record = document.project.source(document.project.order[1]).unwrap();
    let key = SourceKey::capture(second.path(), &second_record.development).unwrap();
    assert!(document.cache().get(&key).is_none());
    let again = request(&document).load(&scheduler, &|| false).unwrap();
    assert!(Arc::ptr_eq(&source, &again.context.source));
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    document.project.roll.exposure = 1.;
    document
        .project
        .source_mut(first_id)
        .unwrap()
        .adjustment_layers = Some(crate::project::AdjustmentLayers::with_global(
        crate::adjustment_layers::AdjustmentLayerSettings {
            exposure: -0.3,
            ..Default::default()
        },
    ));
    let edited = request(&document).load(&scheduler, &|| false).unwrap();
    assert_eq!(
        edited.context.source.reference_exposure.get(),
        Some(&reference)
    );
    assert_eq!(document.project.roll.exposure, 1.);
    assert_eq!(
        document
            .project
            .source(first_id)
            .unwrap()
            .grade_snapshot()
            .unwrap()
            .layers()[0]
            .photo
            .exposure,
        -0.3
    );
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("grade.antitone");
    let saved = document.save(&path, crate::project::Save::New).unwrap();
    assert!(!saved.dirty());
    assert!(saved.project.exposure_metadata.is_none());
    let file_project = crate::project::Project::load(&path).unwrap();
    let cached = file_project.exposure_metadata.unwrap();
    assert_eq!(
        cached.entries.len(),
        1,
        "saving must not meter the unvisited photo"
    );
    assert_eq!(cached.entries[0].reference, reference);
    let reopened = Document::open(&path).unwrap();
    assert!(!reopened.dirty());
    assert!(reopened.project.exposure_metadata.is_none());
    let reacquired = request(&reopened).load(&scheduler, &|| false).unwrap();
    assert_eq!(
        reacquired.context.source.reference_exposure.get(),
        Some(&reference)
    );
    assert!(
        reacquired.camera_proxy.is_none(),
        "saved exposure should not re-sample the RAW"
    );
}

#[test]
fn offscreen_known_reference_is_included_in_read_only_parameter_preflight() {
    let first = preview_fixture::neutral_dng(0.3);
    let second = preview_fixture::neutral_dng(2.25);
    let mut editor = Editor::state();
    editor.input = InputSettings::Digital;
    editor.document.project.input = InputSettings::Digital;
    editor
        .document
        .import(&[first.path().to_owned(), second.path().to_owned()])
        .unwrap();
    editor.current_source = Some(editor.document.project.order[0]);
    let id = editor.document.project.order[1];
    let expected = editor.document.project.source(id).unwrap().clone();
    Request {
        input: InputSettings::Digital,
        path: second.path().to_owned(),
        wb: Some(expected.development.decode.fixed_wb),
        cache: Some(Cache::new(second.path(), Some(&expected), &editor.document)),
        expected: Some(expected),
        correction: None,
    }
    .load(&development::Scheduler::new(), &|| false)
    .unwrap();
    // Completed derived metadata must suffice even with no offscreen pixels or readable file.
    editor.document.cache().set_budget(0);
    std::fs::rename(second.path(), second.path().with_extension("unavailable")).unwrap();
    let (device, queue) = pollster::block_on(async {
        let adapter = crate::gpu::instance()
            .request_adapter(&Default::default())
            .await
            .unwrap();
        adapter.request_device(&Default::default()).await.unwrap()
    });
    editor.initialize_viewer(&device, &queue);
    editor.roll.exposure = 16.;
    editor.document.project.roll = editor.roll;
    let mut locals = crate::project::AdjustmentLayers::default();
    for exposure in [16., 16., 16., 16., 16., 4.] {
        locals
            .add(
                "Range control".into(),
                crate::adjustment_layers::AdjustmentLayer {
                    mask: crate::adjustment_layers::AdjustmentLayerCoverage::Linear {
                        start: crate::geometry::SourcePoint([0., 0.]),
                        end: crate::geometry::SourcePoint([64., 64.]),
                    },
                    photo: crate::adjustment_layers::AdjustmentLayerSettings {
                        exposure,
                        ..Default::default()
                    },
                    enabled: true,
                    invert: false,
                    strength: 1.,
                },
            )
            .unwrap();
    }
    editor
        .document
        .project
        .source_mut(id)
        .unwrap()
        .adjustment_layers = Some(locals);
    editor
        .validate_document_candidate(&editor.document)
        .expect("accepted controls must fit with the known baseline");
    let mut offscreen = editor.document.project.source(id).unwrap().clone();
    offscreen
        .adjustment_layers
        .as_mut()
        .unwrap()
        .add(
            "Range candidate".into(),
            crate::adjustment_layers::AdjustmentLayer::global(
                crate::adjustment_layers::AdjustmentLayerSettings {
                    exposure: 16.,
                    ..Default::default()
                },
            ),
        )
        .unwrap();
    // The declared stack reaches about 2^124 without the baseline; the supported f32
    // bound is 2^126. Adding the measured 2.25 EV alone makes it unsafe.
    crate::editor::processing::validate_source(
        Some(&device),
        InputSettings::Digital,
        editor.roll,
        &offscreen,
    )
    .expect("the zero-start control must fit; only the known baseline makes this stack unsafe");
    let normalized = editor
        .document
        .positive_matrix(&offscreen, InputSettings::Digital)
        .unwrap();
    let native = glam::DMat3::from_cols_array_2d(&offscreen.development.camera_to_ap1);
    assert!(((normalized.x_axis.length() / native.x_axis.length()).log2() - 2.25).abs() < 0.03);
    let before = editor.document.project.clone();
    assert!(editor.validate_source_candidate(&offscreen).is_err());
    let mut candidate = editor.document.clone();
    candidate.project.source_mut(id).unwrap().adjustment_layers = offscreen.adjustment_layers;
    assert!(editor.validate_document_candidate(&candidate).is_err());
    assert_eq!(editor.document.project, before);
    assert!(editor.document.visit.ready.is_none());
}

#[test]
fn save_open_admission_uses_restored_positive_and_negative_source_normalization() {
    use crate::{
        adjustment_layers::{AdjustmentLayer, AdjustmentLayerSettings},
        exposure_reference::StartingExposure,
        project::{AdjustmentLayers, Project, Save},
    };
    let raw = preview_fixture::neutral_dng(1.3);
    let directory = tempfile::tempdir().unwrap();
    for (name, reference_ev, last_layer) in [("attenuated", -3., 8.), ("amplified", 0., 4.)] {
        let mut document = Document::default();
        document.project.input = InputSettings::Digital;
        document.import(&[raw.path().to_owned()]).unwrap();
        let id = document.project.order[0];
        let source = document.project.source(id).unwrap().clone();
        let key = SourceKey::capture(raw.path(), &source.development).unwrap();
        document
            .cache()
            .exposure_memo(&key)
            .set(ReferenceExposure::Matched(StartingExposure {
                ev: reference_ev,
                samples: 1024,
                spread: 0.,
            }))
            .unwrap();
        document.project.roll.exposure = 16.;
        let mut layers = AdjustmentLayers::default();
        for exposure in [16., 16., 16., 16., 16., 16., last_layer] {
            layers
                .add(
                    "Range control".into(),
                    AdjustmentLayer::global(AdjustmentLayerSettings {
                        exposure,
                        ..Default::default()
                    }),
                )
                .unwrap();
        }
        document.project.source_mut(id).unwrap().adjustment_layers = Some(layers);
        let record = document.project.source(id).unwrap();
        let zero_start = crate::editor::processing::validate_source(
            None,
            InputSettings::Digital,
            document.project.roll,
            record,
        );
        if reference_ev < 0. {
            assert!(
                zero_start.is_err(),
                "attenuation is essential for this stack"
            );
        } else {
            zero_start.unwrap();
        }
        let path = directory.path().join(format!("{name}.antitone"));
        document.save(&path, Save::New).unwrap();
        if reference_ev < 0. {
            let opened = Document::open(&path)
                .expect("normalization must be restored before range admission");
            assert_eq!(opened.project.roll, document.project.roll);
            assert_eq!(opened.project.sources, document.project.sources);
            assert!(!opened.dirty());
        } else {
            let mut saved = Project::load(&path).unwrap();
            saved.exposure_metadata.as_mut().unwrap().entries[0].reference =
                ReferenceExposure::Matched(StartingExposure {
                    ev: 3.,
                    samples: 1024,
                    spread: 0.,
                });
            saved.save(&path, Save::Replace).unwrap();
            assert!(
                Document::open(&path).is_err(),
                "restored amplification makes this otherwise supported stack unsafe"
            );
        }
    }
}
