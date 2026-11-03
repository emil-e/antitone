use super::*;
use crate::project::Development;
use crate::{
    adjustment_layers::{AdjustmentLayer, AdjustmentLayerCoverage},
    geometry::SourcePoint,
    gpu::SourceGeometry,
    project::{AdjustmentLayers, SourceDevelopment},
};

pub(in crate::editor) async fn fixture() -> (Editor, wgpu::Device, wgpu::Queue) {
    let instance = crate::gpu::instance();
    let adapter = instance.request_adapter(&Default::default()).await.unwrap();
    let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
    let mut editor = super::super::tests::fixture();
    editor.initialize_viewer(&device, &queue);
    let record = Source {
        id: SourceId(0),
        path: "local-fixture.ARW".into(),
        development: SourceDevelopment {
            decode: Development::new("test-backend".into(), [1.; 4]),
            lens_correction: None,
            dng_development: None,
            dimensions: [150, 100],
            active_area: [120, 73, 23, 19],
            camera_model: "test-camera".into(),
            camera_to_ap1: glam::DMat3::IDENTITY.to_cols_array_2d(),
            input_floor: crate::processing::CAMERA_INPUT_FLOOR,
        },
        reference_only: false,
        adjustment_layers: Some(crate::project::AdjustmentLayers::with_global(
            editor.document.visit.adjustment_layer_settings,
        )),
        repairs: Default::default(),
        orientation: Default::default(),
        crop: None,
    };
    let development = crate::project::Development::new(crate::raw::version(), [1.; 4]);
    let id = editor
        .document
        .register_with_development(record.clone(), development.clone())
        .unwrap();
    editor.document.visit.current_source = Some(id);
    let renderer = crate::renderer::GpuRenderer::new(&device, &queue)
        .await
        .unwrap();
    let proxy = crate::viewer::GpuProxy::new(
        renderer,
        &device,
        [7, 5],
        &[[1100, 900, 750, 0]; 35],
        glam::DMat3::IDENTITY,
        None,
        SourceGeometry::new(record.development.active_area, [7, 5]).unwrap(),
    )
    .unwrap();
    editor.document.visit.canvas.ready = Some(Ready {
        reference: None,
        developed: crate::editor::tests::developed_fixture(),
        file: thumbnails::FileKey::new(&record.path),
        thumbnail: None,
        path: record.path.clone(),
        area: record.development.active_area.into(),
        analysis_support: None,
        proxy,
        record,
    });
    editor.document.source = false;
    editor.tab = 1;
    let mut layers = AdjustmentLayers::default();
    for (name, adjustment) in [
        ("Linear", local(0.7)),
        (
            "Radial",
            AdjustmentLayer {
                mask: AdjustmentLayerCoverage::Radial {
                    center: SourcePoint([131., 82.]),
                    radii: [10., 7.],
                    angle: 0.4,
                    feather: 0.65,
                },
                photo: AdjustmentLayerSettings {
                    contrast: 1.2,
                    saturation: 1.3,
                    ..Default::default()
                },
                ..local(0.)
            },
        ),
    ] {
        layers.add(name.into(), adjustment).unwrap();
    }
    editor
        .document
        .project
        .source_mut(id)
        .unwrap()
        .adjustment_layers = Some(layers);
    editor.document.visit.canvas.last_layout = Some((
        editor.document.visit.canvas.viewport.layout(
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 600.)),
            editor.document.visit.canvas.ready.as_ref().unwrap().area,
            1.,
        ),
        1.,
    ));
    editor.document.visit.adjustment_layer_settings = editor
        .selected_adjustment_layer_settings()
        .unwrap_or_default();
    editor.sync_document();
    (editor, device, queue)
}

fn local(exposure: f64) -> AdjustmentLayer {
    AdjustmentLayer {
        mask: AdjustmentLayerCoverage::Linear {
            start: SourcePoint([121., 74.]),
            end: SourcePoint([142., 91.]),
        },
        photo: AdjustmentLayerSettings {
            exposure,
            ..Default::default()
        },
        enabled: true,
        invert: false,
        strength: 0.8,
    }
}

fn positive(editor: &Editor) -> ProcessingInput {
    editor
        .positive_processing(
            editor.document.input,
            editor.document.roll,
            editor.document.visit.adjustment_layer_settings,
        )
        .unwrap()
}

fn frame(editor: &mut impl crate::editor::tests::UiFixture, context: &egui::Context) {
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100., 800.),
            )),
            ..Default::default()
        },
        |ui| editor.ui(ui),
    );
    output.textures_delta.clear();
}

#[test]
fn captured_grades_follow_processing_not_names_allocator_or_view_state() {
    pollster::block_on(async {
        let (mut editor, _, _) = fixture().await;
        let id = editor.document.visit.current_source.unwrap();
        let first = positive(&editor);
        let unchanged = positive(&editor);
        assert_eq!(first.clone(), unchanged.clone());
        let layers = editor
            .document
            .project
            .source_mut(id)
            .unwrap()
            .adjustment_layers
            .as_mut()
            .unwrap();
        layers.layers[0].name = "Renamed".into();
        layers.next_id += 1;
        editor.source_exposure = 2.;
        editor.document.visit.canvas.viewport.zoom = Some(3.);
        editor.document.target = OutputTarget::Srgb;
        let renamed = positive(&editor);
        assert_eq!(first.clone(), renamed.clone());
        let old_snapshot = first.grade().unwrap().snapshot().clone();
        editor
            .document
            .project
            .source_mut(id)
            .unwrap()
            .adjustment_layers
            .as_mut()
            .unwrap()
            .layers
            .reverse();
        let reordered = positive(&editor);
        assert_ne!(first.clone(), reordered.clone());
        assert_eq!(first.grade().unwrap().snapshot(), &old_snapshot);
        editor.document.visit.adjustment_layer_settings.exposure = 1.;
        let graded = positive(&editor);
        assert_eq!(
            graded
                .grade()
                .unwrap()
                .snapshot()
                .layers()
                .last()
                .unwrap()
                .photo,
            editor.document.visit.adjustment_layer_settings
        );
        assert_ne!(reordered.clone(), graded.clone());
        editor.document.roll.wb = crate::color::Balance {
            warmth: 0.017,
            tint: -0.013,
        };
        let roll_graded = positive(&editor);
        assert_ne!(graded.clone(), roll_graded.clone());
        assert_eq!(
            graded.grade().unwrap().snapshot(),
            roll_graded.grade().unwrap().snapshot()
        );
        let revision = editor
            .document
            .visit
            .canvas
            .ready
            .as_ref()
            .unwrap()
            .proxy
            .image(editor.document.target)
            .unwrap()
            .source_revision();
        assert_eq!(first.grade().unwrap().snapshot(), &old_snapshot);
        assert_eq!(
            editor
                .document
                .visit
                .canvas
                .ready
                .as_ref()
                .unwrap()
                .proxy
                .image(editor.document.target)
                .unwrap()
                .source_revision(),
            revision
        );
    });
}

#[test]
fn developed_floors_are_source_specific_in_editor_preparation_and_offscreen_admission() {
    pollster::block_on(async {
        let (mut editor, device, _) = fixture().await;
        let first = editor.document.visit.current_source.unwrap();
        editor
            .document
            .project
            .source_mut(first)
            .unwrap()
            .adjustment_layers = None;
        let mut shaded = editor.document.state.project.source(first).unwrap().clone();
        shaded.id = SourceId(0);
        shaded.path = "shaded-fixture.ARW".into();
        shaded.development.lens_correction = Some("ab".repeat(32));
        shaded.development.input_floor *= 4.;
        let second = editor.document.state.project.add_source(shaded).unwrap();
        let base = editor.calibration().unwrap();
        assert_eq!(
            editor.processing_calibration(base).unwrap().input_floor,
            base.input_floor
        );
        let source = editor.document.state.project.source(second).unwrap();
        let input = editor
            .positive_processing_for_source(
                base,
                editor.document.roll,
                source,
                AdjustmentLayerSettings::default(),
            )
            .unwrap();
        let expected = ProcessingParameters::positive(
            Calibration {
                input_floor: base.input_floor * 4.,
                ..base
            },
            editor.document.roll,
            glam::DMat3::IDENTITY,
        )
        .unwrap();
        assert_eq!(
            bytemuck::bytes_of(&input.parameters()),
            bytemuck::bytes_of(&expected)
        );
        let near_floor = Calibration {
            dmax: DVec3::splat(base.input_floor * 2.),
            ..base
        };
        assert!(
            editor
                .positive_parameters(near_floor, editor.document.roll)
                .is_ok()
        );
        assert!(
            editor
                .validate_processing(
                    near_floor,
                    editor.document.roll,
                    editor.document.visit.adjustment_layer_settings
                )
                .is_err()
        );
        editor.set_dmax(Some(near_floor.dmax));
        assert!(
            editor
                .validate_source_candidate(editor.document.state.project.source(second).unwrap())
                .is_err()
        );
        editor.document.visit.current_source = Some(second);
        assert_eq!(
            editor.processing_calibration(base).unwrap().input_floor,
            base.input_floor * 4.
        );
        editor.set_dmax(Some(base.dmax));
        editor.sync_document();
        editor.document.state.project.order = vec![second];
        let record = editor
            .document
            .state
            .project
            .source(second)
            .unwrap()
            .clone();
        // A nonbinary bound exposes precision-floor round trips through an already scaled floor.
        let map = Arc::new(
            crate::flat_field::FlatField::from_nodes(
                record.development.active_area,
                [2, 2],
                vec![DVec3::splat(5.235436781787922); 4],
            )
            .unwrap(),
        );
        editor
            .document
            .project
            .set_flat_field(Some(crate::project::FlatFieldSettings::new(&record, map)));
        editor.document.correction = Correction::prepare(&device, &editor.document, None).unwrap();
        let recipe = editor.document.state.project.image_recipe(second).unwrap();
        assert_eq!(
            editor.processing_calibration(base).unwrap(),
            recipe
                .neutral
                .processing_input()
                .unwrap()
                .calibration()
                .unwrap()
        );
    });
}

#[test]
fn correction_changes_captured_processing_without_mutating_earlier_stacks() {
    pollster::block_on(async {
        let (mut editor, device, _) = fixture().await;
        let first = positive(&editor);
        let id = editor.document.visit.current_source.unwrap();
        editor.sync_document();
        let mut candidate = editor.document.capture();
        candidate.project.source_mut(id).unwrap().adjustment_layers = None;
        let source = candidate.project.source(id).unwrap();
        let field = Arc::new(
            crate::flat_field::FlatField::from_nodes(
                source.development.active_area,
                [2, 2],
                vec![DVec3::new(1.1, 0.9, 1.2); 4],
            )
            .unwrap(),
        );
        candidate
            .project
            .set_flat_field(Some(crate::project::FlatFieldSettings::new(
                source,
                field.clone(),
            )));
        editor.document.correction = Correction::prepare(&device, &candidate, None).unwrap();
        let corrected = positive(&editor);
        assert_ne!(first.clone(), corrected.clone());
        let calibration = field
            .processing_calibration(editor.calibration().unwrap())
            .unwrap();
        let expected = ProcessingInput::from(Arc::new(
            crate::gpu::ProcessingInput::new(
                &device.limits(),
                calibration,
                editor.document.roll,
                first.grade().unwrap().snapshot().clone(),
                glam::DMat3::IDENTITY,
                editor.document.visit.canvas.ready.as_ref().unwrap().area,
            )
            .unwrap(),
        ));
        assert_eq!(
            bytemuck::bytes_of(&corrected.parameters()),
            bytemuck::bytes_of(&expected.parameters())
        );
        let global = ProcessingParameters::positive(
            calibration,
            editor.document.roll,
            glam::DMat3::IDENTITY,
        )
        .unwrap();
        assert_eq!(
            bytemuck::bytes_of(&corrected.parameters()),
            bytemuck::bytes_of(&global),
            "layer controls must not leak into upstream Roll/input parameters"
        );
        assert_eq!(corrected.clone(), positive(&editor).clone());
        editor.document.correction = None;
        assert_eq!(positive(&editor).clone(), first.clone());
        editor
            .document
            .project
            .source_mut(id)
            .unwrap()
            .adjustment_layers
            .as_mut()
            .unwrap()
            .layers
            .clear();
        let empty = positive(&editor);
        assert!(empty.grade().is_some());
        assert_eq!(empty.grade().unwrap().snapshot().layers().len(), 0);
        assert_eq!(
            first.grade().unwrap().snapshot().layers().len(),
            2,
            "captured resources remain immutable after deletion"
        );
    });
}

#[test]
fn offscreen_local_stack_blocks_roll_and_calibration_candidates_without_replacing_state() {
    pollster::block_on(async {
        let (mut editor, device, _) = fixture().await;
        let accepted = positive(&editor);
        let id = editor.document.visit.current_source.unwrap();
        let mut other = editor.document.state.project.source(id).unwrap().clone();
        other.id = SourceId(editor.document.state.project.next_source_id);
        other.path = "offscreen.ARW".into();
        let mut layers = AdjustmentLayers::default();
        for exposure in [-16., 16.] {
            for _ in 0..7 {
                layers.add("Stage".into(), local(exposure)).unwrap();
            }
        }
        for record in &mut layers.layers {
            record.adjustment.strength = 1.;
        }
        other.adjustment_layers = Some(layers.clone());
        let mut disabled = other.clone();
        for record in &mut disabled.adjustment_layers.as_mut().unwrap().layers {
            record.adjustment.enabled = false;
        }
        validate_source(
            Some(&device),
            editor.input(),
            editor.document.roll,
            &disabled,
        )
        .unwrap();
        assert!(
            validate_source(Some(&device), editor.input(), editor.document.roll, &other).is_err()
        );
        editor.document.state.project.sources.push(other.clone());
        editor.document.state.project.order.push(other.id);
        editor.document.state.project.next_source_id += 1;
        let before = (
            editor.document.roll,
            editor.document.visit.adjustment_layer_settings,
            editor.dmin(),
            editor.dmax(),
            editor.document.state.project.clone(),
        );
        let generation = editor.document.visit.canvas.grade.original.continuation();
        assert!(
            editor
                .validate_processing(
                    editor.document.input,
                    RollSettings {
                        exposure: 1.,
                        ..editor.document.roll
                    },
                    editor.document.visit.adjustment_layer_settings
                )
                .is_err()
        );
        editor.document.visit.picking.display = Some(Pick::Gamma.display());
        assert!(
            (calibration::set_gamma_balance(
                &mut editor,
                Balance {
                    warmth: 0.1,
                    tint: -0.1,
                }
            ))
            .is_err()
        );
        editor.finish_pick(&crate::application::UiContext::render_only());
        assert_eq!(
            (
                editor.document.roll,
                editor.document.visit.adjustment_layer_settings,
                editor.dmin(),
                editor.dmax(),
                editor.document.state.project.clone()
            ),
            before
        );
        assert_eq!(
            editor.document.visit.canvas.grade.original.continuation(),
            generation
        );
        assert_eq!(accepted.clone(), positive(&editor).clone());
        let proposed_cal = Calibration {
            dmin: editor.calibration().unwrap().dmin * 1.1,
            ..editor.calibration().unwrap()
        };
        assert!(
            editor
                .validate_processing(
                    proposed_cal,
                    editor.document.roll,
                    editor.document.visit.adjustment_layer_settings
                )
                .is_err()
        );
        assert_eq!(accepted.clone(), positive(&editor).clone());
    });
}

#[test]
fn original_comparison_and_scopes_capture_the_same_immutable_stack_and_reject_stale_results() {
    pollster::block_on(async {
        let (mut editor, device, _) = fixture().await;
        editor.document.visit.working_view.compare = true;
        editor.scopes = Scopes::new(&device);
        let id = editor.document.visit.current_source.unwrap();
        let context = egui::Context::default();
        frame(&mut editor, &context);
        let accepted = positive(&editor);
        let (_, scope, _) = editor.scope_input().unwrap();
        assert_eq!(scope.clone(), accepted.clone());
        editor.document.visit.canvas.grade.original.last_edit =
            std::time::Instant::now() - Duration::from_secs(1);
        let request = editor
            .document
            .visit
            .canvas
            .grade
            .original
            .request()
            .unwrap();
        let retired = request.begin().unwrap();
        assert_eq!(request.demand.target, editor.document.target);
        assert!(matches!(
            request.demand.capture,
            working_view::Capture::Photograph(_)
        ));
        let (captured, view) = (
            request
                .demand
                .capture
                .processing(&device.limits(), &request.demand.repairs)
                .unwrap(),
            request.demand.view.unwrap(),
        );
        assert_eq!(captured.clone(), accepted.clone());
        editor
            .document
            .project
            .source_mut(id)
            .unwrap()
            .adjustment_layers
            .as_mut()
            .unwrap()
            .layers[0]
            .adjustment
            .invert = true;
        frame(&mut editor, &context);
        assert_ne!(positive(&editor).clone(), captured.clone());
        assert_ne!(
            editor.document.visit.canvas.grade.original.continuation(),
            retired
        );
        let (_, scope, _) = editor.scope_input().unwrap();
        assert_eq!(scope.clone(), positive(&editor).clone());
        retired.receive(Ok(Some(
            editor
                .document
                .visit
                .canvas
                .ready
                .as_ref()
                .unwrap()
                .proxy
                .image(editor.document.target)
                .unwrap(),
        )));
        assert!(
            editor
                .document
                .visit
                .canvas
                .grade
                .original
                .settled()
                .is_none()
        );
        assert!(!editor.document.visit.canvas.grade.original.refining());
        assert_eq!(
            captured.clone(),
            accepted.clone(),
            "worker resources are immutable after edits"
        );
        editor.document.source = true;
        editor.source_exposure = 2.;
        assert!(editor.scope_input().is_err());
        frame(&mut editor, &context);
        assert!(
            editor.document.visit.canvas.grade.original.wanted.is_none(),
            "source inspection never requests photographic comparison"
        );
        assert!(view.area.width > 0);
    });
}

fn tracked_grade(input: &ProcessingInput) -> (std::sync::Weak<crate::gpu::Grading>, u64) {
    let grading = input.grade_shared().unwrap();
    (Arc::downgrade(grading), grading.byte_len())
}

fn live_grade_snapshots(grades: &[(std::sync::Weak<crate::gpu::Grading>, u64)]) -> (usize, u64) {
    grades
        .iter()
        .filter(|(grade, _)| grade.strong_count() != 0)
        .fold((0, 0), |(count, bytes), (_, size)| {
            (count + 1, bytes + size)
        })
}

#[test]
fn captured_original_mathematical_inputs_follow_active_consumers_not_ui_history() {
    use crate::gpu_scopes::{GpuScopes, ScopePaint};
    use egui_wgpu::CallbackTrait;
    use std::time::Instant;
    pollster::block_on(async {
        let (mut editor, device, queue) = fixture().await;
        editor.document.visit.working_view.compare = true;
        let accepted = positive(&editor);
        let mut tracked = vec![tracked_grade(&accepted)];
        let bytes = tracked[0].1;
        let proxy = editor
            .document
            .visit
            .canvas
            .ready
            .as_ref()
            .unwrap()
            .proxy
            .clone();
        let images = [
            proxy.image(OutputTarget::Srgb).unwrap(),
            proxy.image(OutputTarget::DisplayP3).unwrap(),
        ];
        let image = images[editor.document.target as usize].clone();
        let context = egui::Context::default();
        let gpu = Arc::new(GpuScopes::new(&device).await.unwrap());
        let (_, _, scope_frame) = editor.scope_input().unwrap();
        let image_paint = ImagePaint::with_processing(
            image.clone(),
            Some(accepted.clone()),
            crate::geometry::TextureMapping::IDENTITY,
        )
        .unwrap();
        let scope_paint = ScopePaint::with_processing(
            gpu.clone(),
            image,
            accepted.clone(),
            scope_frame,
            crate::scopes::Settings {
                target: editor.document.target,
                plot: crate::scopes::Plot::Rgb,
                amplification: 1,
            },
            context.clone(),
        )
        .unwrap();
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [1100, 800],
            pixels_per_point: 1.,
        };
        let mut resources = Default::default();
        let mut encoder = device.create_command_encoder(&Default::default());
        image_paint.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        scope_paint.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        for image in &images {
            image
                .prepare_processing(&device, &mut encoder, &accepted)
                .unwrap();
        }
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        drop(accepted);

        let observe = |editor: &mut Editor| {
            let layout = editor.document.visit.canvas.last_layout.unwrap().0;
            let frame = layout.physical_frame_raster(1.);
            editor.document.visit.canvas.grade.original.observe(
                OriginalPurpose::Reference,
                original_view::Demand::fixture(
                    positive(editor).clone(),
                    editor.document.target,
                    Some(ViewRequest {
                        area: frame.bounds().unwrap(),
                        output: frame.output,
                        frame: Some(frame),
                    }),
                    layout.composition,
                    editor.source_context().unwrap(),
                ),
            );
        };
        observe(&mut editor);
        editor.document.visit.canvas.grade.original.last_edit =
            Instant::now() - Duration::from_secs(1);
        // Retain consumer input independently of newer UI demand. Actual task and
        // running-job admission are covered by the live workflow tests.
        let held = editor
            .document
            .visit
            .canvas
            .grade
            .original
            .request()
            .unwrap();
        let held_grade = tracked_grade(
            &held
                .demand
                .capture
                .processing(&device.limits(), &held.demand.repairs)
                .unwrap(),
        )
        .0;
        let mut requested_grades = vec![(held_grade.clone(), bytes)];
        for i in 1..=40 {
            editor.document.visit.adjustment_layer_settings.exposure = f64::from(i) / 100.;
            observe(&mut editor);
            editor.document.visit.canvas.grade.original.last_edit =
                Instant::now() - Duration::from_secs(1);
            let latest = editor
                .document
                .visit
                .canvas
                .grade
                .original
                .request()
                .unwrap();
            let latest_input = latest
                .demand
                .capture
                .processing(&device.limits(), &latest.demand.repairs)
                .unwrap();
            assert_eq!(latest_input, positive(&editor));
            let latest_grade = tracked_grade(&latest_input).0;
            requested_grades.push((latest_grade.clone(), bytes));
            drop(latest_input);
            assert!(held_grade.upgrade().is_some());
            assert_eq!(live_grade_snapshots(&tracked), (1, bytes));
            drop(latest);
            assert!(
                latest_grade.upgrade().is_some(),
                "the current request remains an independent consumer"
            );
            assert_eq!(
                live_grade_snapshots(&requested_grades),
                (2, 2 * bytes),
                "only held work and current demand retain requested mathematical inputs"
            );
        }
        let newest = editor
            .document
            .visit
            .canvas
            .grade
            .original
            .request()
            .unwrap();
        tracked.push(tracked_grade(
            &newest
                .demand
                .capture
                .processing(&device.limits(), &newest.demand.repairs)
                .unwrap(),
        ));
        assert_ne!(newest.completion(), held.completion());
        assert!(held.completion().receive(Ok(None)).is_none());
        assert!(
            editor
                .document
                .visit
                .canvas
                .grade
                .original
                .settled()
                .is_none()
        );
        drop(held);
        assert!(held_grade.upgrade().is_none());

        editor.document.visit.canvas.grade.original.wanted = None;
        editor.document.visit.canvas.grade = Default::default();
        assert_eq!(live_grade_snapshots(&tracked), (2, 2 * bytes));
        drop(newest);
        assert_eq!(live_grade_snapshots(&tracked), (1, bytes));
        drop(image_paint);
        drop(scope_paint);
        gpu.invalidate();
        assert_eq!(
            live_grade_snapshots(&tracked),
            (1, bytes),
            "image preparation independently retains the old signature"
        );
        let inspection = ProcessingInput::from(ProcessingParameters::source());
        let mut encoder = device.create_command_encoder(&Default::default());
        images[0]
            .prepare_processing(&device, &mut encoder, &inspection)
            .unwrap();
        assert_eq!(live_grade_snapshots(&tracked), (1, bytes));
        images[1]
            .prepare_processing(&device, &mut encoder, &inspection)
            .unwrap();
        assert_eq!(live_grade_snapshots(&tracked), (0, 0));
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        println!(
            "local_grade_retention edits=40 pending_original_demand=1 peak_arc_owned_grade_bytes={} released_arc_owned_grade_bytes={} excludes=GPU_and_other_snapshot_storage",
            2 * bytes,
            2 * bytes
        );
    });
}

#[test]
fn preparing_other_photos_does_not_retain_unreferenced_grade_snapshots() {
    pollster::block_on(async {
        let (editor, _, _) = fixture().await;
        let calibration = editor.calibration().unwrap();
        let record = editor
            .document
            .project
            .source(editor.document.visit.current_source.unwrap())
            .unwrap();
        let mut tracked = Vec::new();
        let mut held = None;
        for i in 0..36 {
            let mut source = record.clone();
            source.id = SourceId(i);
            let input = editor
                .positive_processing_for_source(
                    calibration,
                    editor.document.roll,
                    &source,
                    editor.document.visit.adjustment_layer_settings,
                )
                .unwrap();
            tracked.push(tracked_grade(&input));
            if i == 35 {
                held = Some(input);
            }
        }
        let bytes = tracked[0].1;
        assert_eq!(
            live_grade_snapshots(&tracked),
            (1, bytes),
            "only the explicitly held photograph owns its snapshot"
        );
        drop(held);
        assert_eq!(live_grade_snapshots(&tracked), (0, 0));
    });
}

#[test]
fn document_replacement_keeps_active_inputs_and_device_infrastructure_not_hidden_grades() {
    pollster::block_on(async {
        let (mut editor, _, _) = fixture().await;
        let input = positive(&editor);
        let grade = tracked_grade(&input).0;
        editor.sync_document();
        let captured = editor.document.capture();
        let id = editor.document.visit.current_source;
        editor.document = editor.document.replacement(captured);
        assert!(
            grade.upgrade().is_some(),
            "an active consumer owns its captured input independently"
        );
        assert!(editor.document.visit.working_view.device().is_some());
        drop(input);
        assert!(grade.upgrade().is_none());
        editor.document.visit.current_source = id;
        assert!(positive(&editor).grade().is_some());
    });
}
