use super::*;
use crate::project::Development;
use crate::{application::test_support::with_application, project::SourceDevelopment};

fn record(path: &str) -> Source {
    Source {
        id: SourceId(0),
        path: path.into(),
        development: SourceDevelopment {
            decode: Development::new("test-backend".into(), [1.; 4]),
            dimensions: [824, 624],
            active_area: [12, 12, 800, 600],
            camera_model: "test-camera".into(),
            camera_to_ap1: glam::DMat3::IDENTITY.to_cols_array_2d(),
        },
        reference_only: false,
        adjustment_layers: Some(crate::project::AdjustmentLayers::with_global(
            Default::default(),
        )),
        repairs: Default::default(),
        orientation: Default::default(),
        crop: None,
    }
}

#[test]
fn document_replacement_retires_unpolled_correction_resources_without_a_late_error() {
    for replace in [false, true] {
        let mut editor = Editor::state();
        let id = editor.document.register(record("photo.ARW")).unwrap();
        editor.document.project.reset_scan_basis();
        let source = editor.document.project.source(id).unwrap();
        let map = Arc::new(
            crate::flat_field::FlatField::from_nodes(
                source.development.active_area,
                [2, 2],
                vec![DVec3::ONE; 4],
            )
            .unwrap(),
        );
        let settings = crate::project::FlatFieldSettings::new(source, map);
        editor.document.project.set_flat_field(Some(settings));
        with_application(editor, |app| {
            app.enter(|editor, cx| {
                editor.restore_correction(cx);
                if replace {
                    editor.new_project();
                }
            });
            app.pump();
            app.read(|editor| {
                assert_eq!(
                    editor.document.correction.error().is_none(),
                    replace,
                    "the surviving owner reports missing GPU resources; a retired owner cannot"
                );
                assert!(!editor.document.correction.pending());
                assert!(editor.document.correction.prepared().is_none());
                assert_eq!(
                    editor.document.project.active_flat_field().is_none(),
                    replace
                );
                assert!(!editor.interaction_blocked() && editor.activities.view().is_none());
            });
        });
    }
}

#[test]
fn removed_reference_preview_does_not_reuse_a_deleted_record() {
    let mut editor = Editor::state();
    let reference = editor.document.register(record("light.ARW")).unwrap();
    editor
        .document
        .project
        .remove_from_filmstrip(reference)
        .unwrap();
    assert!(editor.document.state.project.source(reference).is_err());

    editor.document.state.project.input = crate::processing::InputSettings::Digital;
    let request = editor.reference_preview_input("light.ARW".into());
    assert_eq!(request.wb, None);
    assert!(request.expected.is_none());

    editor.document.state.project.input = crate::project::Project::default().input;
    let request = editor.reference_preview_input("light.ARW".into());
    assert_eq!(request.wb, Some(crate::project::UNITY_WB));
    assert!(request.expected.is_none());
}

#[test]
fn reference_registration_does_not_add_a_photo_or_hide_an_existing_one() {
    let mut document = Document::default();
    let development = Development::new(crate::raw::version(), [1.; 4]);
    let photo = document
        .register_with_development(record("photo.ARW"), development.clone())
        .unwrap();
    let same = document
        .register_reference_with_development(record("photo.ARW"), development.clone())
        .unwrap();
    assert_eq!(same, photo);
    assert_eq!(document.project.order, vec![photo]);
    assert!(!document.project.source(photo).unwrap().reference_only);
    let reference = document
        .register_reference_with_development(record("light.ARW"), development)
        .unwrap();
    assert_eq!(document.project.order, vec![photo]);
    assert!(document.project.source(reference).unwrap().reference_only);
}

#[test]
fn reference_rejection_and_cancel_preserve_document_and_working_context() {
    let mut editor = Editor::state();
    editor.document.source = false;
    editor.document.project.roll.exposure = 2.123456789;
    editor.set_dmin(Some(DVec3::splat(0.2)));
    let original = editor.document.state.project.clone();
    let roll = editor.document.project.roll;
    with_application(editor, |app| {
        app.enter(|e, cx| e.begin_flat_edit("light.ARW".into(), None, cx));
        assert!(
            app.read(|e| e.interaction_blocked() && e.document.flat_edit().unwrap().task.is_some())
        );
        app.run_until(Duration::from_secs(5), |e| !e.interaction_blocked());
        app.read(|e| {
            assert!(
                !e.document.source && e.test_pick().is_none() && e.document.flat_edit().is_none()
            );
            assert_eq!(e.document.state.project, original);
            assert_eq!(
                e.dialogs.message().unwrap().title,
                FailureCase::FlatReference.title()
            );
            assert_eq!(
                e.dialogs.message().unwrap().details,
                "Reference services unavailable"
            );
        });
        app.enter(|e, cx| {
            e.begin_flat_edit("other.ARW".into(), None, cx);
            assert!(!e.interaction_blocked() && e.document.flat_edit().is_none());
            assert_eq!(
                e.dialogs.message().unwrap().details,
                "Reference services unavailable"
            );
            e.dialogs.answer_for_test("OK");
            e.begin_flat_edit("other.ARW".into(), None, cx);
            assert!(e.interaction_blocked());
            e.end_tool();
            assert_eq!(e.document.project.roll, roll);
            assert_eq!(e.document.state.project, original);
            assert!(!e.document.source);
        });
        app.pump();
    });
}

#[test]
fn failed_map_acceptance_restores_transient_edit_without_accepting_document_changes() {
    let mut editor = crate::editor::tests::authored_fixture(&["illuminated-patches.dng"]);
    editor.document.source = false;
    let original = editor.document.state.project.clone();
    let undo = editor
        .history_label(crate::editor::history::Direction::Undo)
        .map(str::to_owned);
    let pixels = editor
        .document
        .visit
        .canvas
        .ready
        .as_ref()
        .unwrap()
        .developed
        .clone();
    let reference =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/bare-light.dng");
    with_application(editor, |app| {
        app.enter(|e, cx| e.begin_flat_edit(reference, None, cx));
        app.run_until(Duration::from_secs(10), |e| !e.interaction_blocked());
        app.enter(|e, cx| {
            let area = e.document.visit.canvas.ready.as_ref().unwrap().area;
            e.set_test_pick_region(Some(Coverage::rectangle(Rect {
                x: area.x,
                y: area.y,
                width: 2,
                height: 2,
            })));
            e.apply_flat_edit(cx);
            assert!(e.document.flat_correction.is_some());
        });
        app.run_until(Duration::from_secs(10), |e| !e.interaction_blocked());
        app.read(|e| {
            assert!(e.document.flat_edit().is_none() && e.test_pick().is_none());
            assert_eq!(e.document.state.project, original);
            assert!(Arc::ptr_eq(
                &pixels,
                &e.document.visit.canvas.ready.as_ref().unwrap().developed
            ));
            assert!(!e.document.source);
            assert_eq!(
                e.dialogs.message().unwrap().title,
                FailureCase::FlatCorrection.title()
            );
            assert_eq!(
                e.history_label(crate::editor::history::Direction::Undo),
                undo.as_deref()
            );
        });
    });
}

#[test]
fn flat_support_uses_source_space_and_restores_rotated_composition_and_view() {
    let mut editor = Editor::state();
    super::super::tests::add_ready_fixture(&mut editor);
    editor.set_dmin(Some(DVec3::splat(0.2)));
    editor.set_dmax(Some(DVec3::splat(0.01)));
    editor.document.source = false;
    editor.document.project.orientation.turn_clockwise();
    editor.document.project.orientation.horizontal = true;
    editor.photo_orientation_mut().turn_counterclockwise();
    editor.photo_orientation_mut().flips.vertical = true;
    *editor.photo_crop_mut() = Some(Crop {
        center: [400.25, 200.75],
        size: [300, 200],
        angle: 0.2,
    });
    editor.document.visit.canvas.viewport.zoom = Some(1.375);
    let composition = Composition::with_crop(
        editor.document.visit.canvas.ready.as_ref().unwrap().area,
        editor.document.project.orientation,
        editor.photo_orientation(),
        editor.photo_crop(),
    )
    .unwrap();
    let canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(180., 120.));
    let layout = editor
        .document
        .visit
        .canvas
        .viewport
        .layout_composition(canvas, composition, 1.);
    editor
        .document
        .visit
        .canvas
        .viewport
        .pan(layout, 1., egui::vec2(24., -13.));
    editor.document.visit.canvas.last_layout = Some((
        editor
            .document
            .visit
            .canvas
            .viewport
            .layout_composition(canvas, composition, 1.),
        1.,
    ));
    let original = editor.document.state.project.clone();
    let viewport = editor.document.visit.canvas.viewport;
    let working_layout = editor.document.visit.canvas.last_layout;
    let ready = editor.document.visit.canvas.ready.as_ref().unwrap();
    let area = ready.area;
    let mut reference = ready.clone();
    reference.record.path = "light.ARW".into();
    reference.record.reference_only = true;
    reference.path = reference.record.path.clone();
    reference.file = thumbnails::FileKey::new(&reference.path);
    let region = Rect {
        x: 37,
        y: 26,
        width: 540,
        height: 320,
    };
    super::super::tests::paused_reference_fixture(&mut editor, reference, Some(region));
    let context = egui::Context::default();
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100., 800.),
            )),
            ..Default::default()
        },
        |ui| crate::editor::tests::paint_controller(&mut editor, ui),
    );
    output.textures_delta.clear();
    let layout = editor.canvas_frame.layout.unwrap().0;
    let layout_state = |saved: Option<(crate::viewport::CanvasLayout, f32)>| {
        saved.map(|(layout, ppp)| {
            (
                layout.composition,
                layout.canvas,
                layout.image,
                layout.scale,
                ppp,
            )
        })
    };
    assert_eq!(
        layout_state(editor.document.visit.canvas.last_layout),
        layout_state(working_layout)
    );
    assert_eq!(
        layout.composition,
        Composition::new(
            area,
            RollOrientation::default(),
            PhotoOrientation::default()
        )
        .unwrap()
    );
    assert_eq!(
        layout.frame_bounds(editor.test_pick_region().unwrap()),
        (
            crate::geometry::FramePoint([25., 19.]),
            crate::geometry::FramePoint([565., 339.])
        )
    );
    assert_eq!(editor.document.state.project, original);
    editor.set_test_pick_region(Some(editor.photo_crop().unwrap().coverage().unwrap()));
    editor.apply_flat_edit(&UiContext::render_only());
    assert!(!editor.interaction_blocked() && editor.document_pending.is_none());
    assert_eq!(editor.document.state.project, original);
    editor.set_test_pick_region(Some(Coverage::rectangle(region)));
    assert_eq!(
        editor.test_pick_region().unwrap().exact_rectangle(area),
        Some(region)
    );
    with_application(editor, |app| {
        app.enter(|e, cx| {
            e.dialogs.answer_for_test("OK");
            e.end_tool();
            assert_eq!(e.document.state.project, original);
            assert_eq!(
                e.photo_crop(),
                original
                    .source(e.document.visit.current_source.unwrap())
                    .unwrap()
                    .crop
            );
            assert_eq!(e.document.project.orientation, original.orientation);
            assert_eq!(e.document.visit.canvas.viewport, viewport);
            assert!(!e.document.source && e.document.flat_edit().is_none());
            assert_eq!(e.normal_canvas_composition(area), composition);
            assert!(
                e.canvas_frame.layout.is_none(),
                "retirement cannot leave reference navigation metadata"
            );
            e.begin_crop();
            let request = e.id;
            e.begin_flat_edit("another-light.ARW".into(), None, cx);
            assert_eq!(e.id, request);
            assert!(e.document.crop_edit().is_none() && e.document.flat_edit().is_some());
            e.finish_flat_edit();
        });
        app.pump();
    });
}

#[test]
fn authored_reference_worker_cancels_applies_and_restores_without_reference_reread() {
    pollster::block_on(async {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw");
        let instance = crate::gpu::instance();
        let adapter = instance
            .request_adapter(&Default::default())
            .await
            .expect("authored correction worker test requires Metal");
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let mut editor = Editor::new(&device, &queue);
        editor.document.thumbnails =
            (Thumbnails::default()).with_sources(editor.document.cache.clone());
        with_application(editor, |app| {
            app.enter(|e, cx| e.add_raws(vec![root.join("illuminated-patches.dng")], cx));
            app.run_until(Duration::from_secs(60), |e| !e.interaction_blocked());
            assert!(app.read(|e| e.document.visit.canvas.ready.is_some()));
            let selected = app.read(|e| e.document.visit.current_source);
            let mut scan = Scan::open(&root.join("illuminated-patches.dng"), None).unwrap();
            let (rgb, original, selected_layer) = app.update(|e| {
                e.set_dmin(Some(
                    scan.reference(Rect {
                        x: 16,
                        y: 16,
                        width: 32,
                        height: 32,
                    })
                    .unwrap(),
                ));
                e.set_dmax(Some(
                    scan.reference(Rect {
                        x: 80,
                        y: 16,
                        width: 32,
                        height: 32,
                    })
                    .unwrap(),
                ));
                let mut layers =
                    crate::project::AdjustmentLayers::with_global(AdjustmentLayerSettings {
                        exposure: 0.7,
                        ..AdjustmentLayerSettings::default()
                    });
                let selected_layer = layers.layers[0].id;
                layers
                    .add(
                        "Finishing".into(),
                        crate::adjustment_layers::AdjustmentLayer::global(
                            AdjustmentLayerSettings {
                                exposure: -0.4,
                                contrast: 0.8,
                                ..AdjustmentLayerSettings::default()
                            },
                        ),
                    )
                    .unwrap();
                e.document
                    .state
                    .project
                    .source_mut(e.document.visit.current_source.unwrap())
                    .unwrap()
                    .adjustment_layers = Some(layers);
                e.refresh_adjustment_layer_selection();
                e.select_adjustment_layer(adjustment_layers::Target::Layer(selected_layer));
                (
                    (e.dmin(), e.dmax()),
                    e.document.state.project.clone(),
                    selected_layer,
                )
            });
            app.enter(|e, cx| {
                e.begin_flat_edit(root.join("bare-light.dng"), None, cx);
                assert_eq!(
                    e.adjustment_layer_target(),
                    adjustment_layers::Target::Layer(selected_layer)
                );
                assert_eq!(e.document.state.project, original);
            });
            app.run_until(Duration::from_secs(60), |e| !e.interaction_blocked());
            app.update(|e| {
                assert_eq!(e.document.state.project, original);
                assert!(e.current_tool().unwrap().is::<FlatTool>());
                assert!(e.test_pick_region().is_some());
                e.end_tool();
                assert_eq!(e.document.state.project, original);
                assert_eq!(e.document.visit.current_source, selected);
                assert_eq!(
                    e.adjustment_layer_target(),
                    adjustment_layers::Target::Layer(selected_layer)
                );
            });
            app.enter(|e, cx| e.begin_flat_edit(root.join("bare-light.dng"), None, cx));
            app.run_until(Duration::from_secs(60), |e| !e.interaction_blocked());
            app.enter(|e, cx| e.apply_flat_edit(cx));
            app.run_until(Duration::from_secs(60), |e| !e.interaction_blocked());
            let settings = app.read(|e| {
                assert!(e.document.correction.prepared().is_some(), "{}", e.status);
                assert_eq!(
                    e.adjustment_layer_target(),
                    adjustment_layers::Target::Layer(selected_layer)
                );
                assert_eq!(
                    e.document
                        .state
                        .project
                        .source(selected.unwrap())
                        .unwrap()
                        .adjustment_layers,
                    original
                        .source(selected.unwrap())
                        .unwrap()
                        .adjustment_layers
                );
                assert_eq!(e.document.state.project.order, original.order);
                assert_eq!((e.dmin(), e.dmax()), rgb);
                assert_eq!(e.document.visit.current_source, selected);
                let settings = e.document.state.project.flat_field().cloned().unwrap();
                assert!(
                    e.document
                        .state
                        .project
                        .source(settings.reference)
                        .unwrap()
                        .reference_only
                );
                settings
            });
            let directory = tempfile::tempdir().unwrap();
            let accepted = app.read(|e| e.document.state.project.clone());
            app.enter(|e, cx| e.restore_history(history::Direction::Undo, cx));
            app.run_until(Duration::from_secs(60), |e| !e.interaction_blocked());
            app.read(|e| {
                assert_eq!(e.document.state.project, original, "{}", e.status);
                assert!(e.document.correction.prepared().is_none());
            });
            app.enter(|e, cx| e.restore_history(history::Direction::Redo, cx));
            app.run_until(Duration::from_secs(60), |e| !e.interaction_blocked());
            app.read(|e| {
                assert_eq!(e.document.state.project, accepted, "{}", e.status);
                assert_eq!(
                    e.document.correction.prepared().unwrap().settings.map,
                    settings.map
                );
            });
            app.enter(|e, cx| e.begin_flat_edit(directory.path().join("missing.dng"), None, cx));
            assert!(app.read(
                |e| e.interaction_blocked() && e.document.flat_edit().unwrap().task.is_some()
            ));
            app.run_until(Duration::from_secs(60), |e| !e.interaction_blocked());
            app.read(|e| {
                assert!(e.status.contains("open"), "{}", e.status);
                assert!(
                    e.document.flat_edit().is_none()
                        && e.test_pick().is_none()
                        && e.document_pending.is_none()
                );
                assert_eq!(e.document.state.project, accepted);
                assert_eq!(e.document.visit.current_source, selected);
                assert!(e.document.visit.canvas.ready.is_some());
            });
            let path = directory.path().join("authored-correction.antitone.json");
            app.enter(|e, cx| e.save_project(path.clone(), crate::project::Save::New, None, cx));
            app.run_until(Duration::from_secs(60), |e| !e.interaction_blocked());
            assert!(!app.read(|e| e.document.dirty()));
            app.enter(|e, cx| e.open_project(path, cx));
            app.run_until(Duration::from_secs(60), |e| {
                !e.files.opening()
                    && !e.document.correction.pending()
                    && !e.document.visit.source.pending()
            });
            app.read(|e| {
                assert_eq!(e.document.state.project, accepted);
                assert_eq!(
                    e.document.correction.prepared().unwrap().settings.map,
                    settings.map
                );
            });
            let mut missing = accepted;
            missing.source_mut(settings.reference).unwrap().path =
                directory.path().join("unavailable.dng");
            let path = directory.path().join("missing-reference.antitone.json");
            missing.save(&path, crate::project::Save::New).unwrap();
            app.enter(|e, cx| e.open_project(path, cx));
            app.run_until(Duration::from_secs(60), |e| {
                !e.files.opening()
                    && !e.document.correction.pending()
                    && !e.document.visit.source.pending()
            });
            app.read(|e| {
                assert_eq!(e.document.state.project, missing);
                assert_eq!(
                    e.document.correction.prepared().unwrap().settings.map,
                    settings.map
                );
                assert!(e.document.visit.canvas.ready.is_some(), "{}", e.status);
            });
        });
    });
}
