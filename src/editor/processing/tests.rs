use super::*;
use crate::gpu::parameters::ProcessingParameters;
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::project::Development;
use crate::{
    adjustment_layers::{AdjustmentLayer, AdjustmentLayerCoverage},
    geometry::SourcePoint,
    gpu::source::SourceGeometry,
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
            editor.photo_settings(),
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
    *editor.photo_settings_mut() = editor
        .selected_adjustment_layer_settings()
        .unwrap_or_default();
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

fn positive(editor: &Editor) -> AuthoredPhotograph {
    editor
        .positive_processing(
            editor.document.project.input,
            editor.document.project.roll,
            editor.photo_settings(),
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
        let proxy = editor
            .document
            .visit
            .canvas
            .ready
            .as_ref()
            .unwrap()
            .proxy
            .clone();
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
        editor.document.set_target(OutputTarget::Srgb);
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
        editor.photo_settings_mut().exposure = 1.;
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
            editor.photo_settings()
        );
        assert_ne!(reordered.clone(), graded.clone());
        editor.document.project.roll.wb = crate::color::Balance {
            warmth: 0.017,
            tint: -0.013,
        };
        let roll_graded = positive(&editor);
        assert_ne!(graded.clone(), roll_graded.clone());
        assert_eq!(
            graded.grade().unwrap().snapshot(),
            roll_graded.grade().unwrap().snapshot()
        );
        assert_eq!(first.grade().unwrap().snapshot(), &old_snapshot);
        assert!(proxy.same_source(&editor.document.visit.canvas.ready.as_ref().unwrap().proxy));
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
                editor.document.project.roll,
                source,
                AdjustmentLayerSettings::default(),
            )
            .unwrap();
        let expected = ProcessingParameters::positive(
            Calibration {
                input_floor: base.input_floor * 4.,
                ..base
            },
            editor.document.project.roll,
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
                .positive_parameters(near_floor, editor.document.project.roll)
                .is_ok()
        );
        assert!(
            editor
                .validate_processing(
                    near_floor,
                    editor.document.project.roll,
                    editor.photo_settings()
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
        let settings = crate::project::FlatFieldSettings::new(source, field.clone());
        candidate.project.set_flat_field(Some(settings));
        editor.document.correction = Correction::prepare(&device, &candidate, None).unwrap();
        let corrected = positive(&editor);
        assert_ne!(first.clone(), corrected.clone());
        let calibration = field
            .processing_calibration(editor.calibration().unwrap())
            .unwrap();
        let expected = AuthoredPhotograph::from(Arc::new(
            crate::gpu::test_inputs::AuthoredPhotograph::new(
                &device.limits(),
                calibration,
                editor.document.project.roll,
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
            editor.document.project.roll,
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
            editor.document.project.roll,
            &disabled,
        )
        .unwrap();
        assert!(
            validate_source(
                Some(&device),
                editor.input(),
                editor.document.project.roll,
                &other
            )
            .is_err()
        );
        editor.document.state.project.sources.push(other.clone());
        editor.document.state.project.order.push(other.id);
        editor.document.state.project.next_source_id += 1;
        let before = (
            editor.document.project.roll,
            editor.photo_settings(),
            editor.dmin(),
            editor.dmax(),
            editor.document.state.project.clone(),
        );
        let generation = editor.document.visit.canvas.original.continuation();
        assert!(
            editor
                .validate_processing(
                    editor.document.project.input,
                    RollSettings {
                        exposure: 1.,
                        ..editor.document.project.roll
                    },
                    editor.photo_settings()
                )
                .is_err()
        );
        picking::preview(&mut editor, PickerCase::Gamma.display());
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
                editor.document.project.roll,
                editor.photo_settings(),
                editor.dmin(),
                editor.dmax(),
                editor.document.state.project.clone()
            ),
            before
        );
        assert_eq!(
            editor.document.visit.canvas.original.continuation(),
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
                    editor.document.project.roll,
                    editor.photo_settings()
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
        editor.document.visit.canvas.original.last_edit =
            std::time::Instant::now() - Duration::from_secs(1);
        let request = editor.document.visit.canvas.original.request().unwrap();
        let retired = request.begin().unwrap();
        assert_eq!(request.demand.target, editor.document.target());
        assert!(matches!(
            request.demand.capture,
            crate::gpu::photograph::request::PhotographRequest::Recipe(_)
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
            editor.document.visit.canvas.original.continuation(),
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
                .image(editor.document.target())
                .unwrap(),
        )));
        assert!(editor.document.visit.canvas.original.settled().is_none());
        assert!(!editor.document.visit.canvas.original.refining());
        assert_eq!(
            captured.clone(),
            accepted.clone(),
            "worker resources are immutable after edits"
        );
        editor.document.source = true;
        editor.source_exposure = 2.;
        assert!(
            editor.scope_input().unwrap().1.grade().is_some(),
            "before repaint, scopes still describe the painted positive"
        );
        frame(&mut editor, &context);
        assert!(
            editor.scope_input().unwrap().1.grade().is_none(),
            "the source repaint supplies source scopes"
        );
        assert!(
            editor.document.visit.canvas.original.wanted.is_none(),
            "source inspection never requests photographic comparison"
        );
        assert!(view.area.width > 0);
    });
}

#[test]
fn captured_calculations_and_recipes_follow_active_consumers_not_ui_history() {
    use crate::gpu::evaluation::ImageNode;
    use crate::gpu::photograph::request::PhotographRequest;
    use crate::gpu_scopes::{GpuScopes, ScopePaint};
    use crate::image_recipe::ImageRecipe;
    use egui_wgpu::CallbackTrait;
    use std::{sync::Weak, time::Instant};

    pollster::block_on(async {
        let (mut editor, device, queue) = fixture().await;
        editor.document.visit.working_view.compare = true;
        let recipe = Arc::new(
            editor
                .positive_recipe(
                    editor.input(),
                    editor.document.project.roll,
                    editor.photo_settings(),
                )
                .unwrap(),
        );
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
        let roots = [
            images[0].recipe_calculation(&recipe, None).unwrap(),
            images[1].recipe_calculation(&recipe, None).unwrap(),
        ];
        let tracked: Vec<Weak<ImageNode>> = roots.iter().map(Arc::downgrade).collect();
        let live_calculations = || {
            tracked
                .iter()
                .filter(|root| root.strong_count() != 0)
                .count()
        };
        let bytes = crate::gpu::grading::Grading::captured_bytes(&roots[0]);
        assert!(bytes > 0, "the fixture must capture photographic arguments");
        let context = egui::Context::default();
        let gpu = Arc::new(GpuScopes::new(&device).await.unwrap());
        let index = editor.document.target() as usize;
        let source = editor
            .document
            .project
            .source(editor.document.visit.current_source.unwrap())
            .unwrap();
        let scope_frame =
            crate::frame_raster::FrameRaster::full(editor.source_composition(source)).unwrap();
        let image_paint = ImagePaint::with_calculation(
            images[index].clone(),
            Some(roots[index].clone()),
            recipe.luts.clone(),
            crate::geometry::TextureMapping::IDENTITY,
        )
        .unwrap();
        let scope_paint = ScopePaint::with_calculation(
            gpu.clone(),
            images[index].clone(),
            (roots[index].clone(), recipe.luts.clone()),
            scope_frame,
            crate::scopes::Settings {
                target: editor.document.target(),
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
        for (image, root) in images.iter().zip(&roots) {
            image
                .prepare_calculation(&device, &mut encoder, root, &recipe.luts)
                .unwrap();
        }
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        drop(roots);
        drop(recipe);

        let observe = |editor: &mut Editor| {
            let layout = editor.document.visit.canvas.last_layout.unwrap().0;
            let frame = layout.physical_frame_raster(1.);
            let recipe = editor
                .positive_recipe(
                    editor.input(),
                    editor.document.project.roll,
                    editor.photo_settings(),
                )
                .unwrap();
            editor.document.visit.canvas.original.observe(
                OriginalPurpose::Reference,
                original_view::Demand {
                    capture: PhotographRequest::Recipe(Arc::new(recipe)),
                    repairs: Default::default(),
                    target: editor.document.target(),
                    view: Some(ViewRequest {
                        area: frame.bounds().unwrap(),
                        output: frame.output,
                        frame: Some(frame),
                    }),
                    composition: layout.composition,
                    source: editor.source_context().unwrap(),
                },
            );
            editor.document.visit.canvas.original.last_edit =
                Instant::now() - Duration::from_secs(1);
        };
        let track_recipe = |request: &original_view::Request| -> Weak<ImageRecipe> {
            match &request.demand.capture {
                PhotographRequest::Recipe(recipe) => Arc::downgrade(recipe),
                _ => panic!("original demand must capture photographic intent"),
            }
        };
        let live_recipes = |recipes: &[Weak<ImageRecipe>]| {
            recipes
                .iter()
                .filter(|recipe| recipe.strong_count() != 0)
                .count()
        };
        observe(&mut editor);
        // Keep a consumer independently of current demand; workflow tests exercise actual jobs.
        let held = editor.document.visit.canvas.original.request().unwrap();
        let held_recipe = track_recipe(&held);
        let mut requested = vec![held_recipe.clone()];
        for i in 1..=40 {
            editor.photo_settings_mut().exposure = f64::from(i) / 100.;
            observe(&mut editor);
            let latest = editor.document.visit.canvas.original.request().unwrap();
            let latest_recipe = track_recipe(&latest);
            let expected = editor
                .positive_recipe(
                    editor.input(),
                    editor.document.project.roll,
                    editor.photo_settings(),
                )
                .unwrap();
            assert_eq!(*latest_recipe.upgrade().unwrap(), expected);
            requested.push(latest_recipe.clone());
            assert!(held_recipe.upgrade().is_some());
            assert_eq!(
                live_calculations(),
                2,
                "edits cannot mutate or withdraw prepared consumers"
            );
            drop(latest);
            assert!(
                latest_recipe.upgrade().is_some(),
                "current demand is an independent consumer"
            );
            assert_eq!(
                live_recipes(&requested),
                2,
                "only held work and current demand retain recipe captures"
            );
        }
        let newest = editor.document.visit.canvas.original.request().unwrap();
        assert_ne!(newest.completion(), held.completion());
        assert!(held.completion().receive(Ok(None)).is_none());
        assert!(editor.document.visit.canvas.original.settled().is_none());
        drop(held);
        assert!(held_recipe.upgrade().is_none());

        editor.document.visit.canvas.original.wanted = None;
        editor.document.visit.canvas.original = Default::default();
        assert_eq!(
            live_recipes(&requested),
            1,
            "the outstanding consumer owns its capture after UI retirement"
        );
        drop(newest);
        assert_eq!(live_recipes(&requested), 0);
        drop(image_paint);
        drop(scope_paint);
        gpu.invalidate();
        assert_eq!(
            live_calculations(),
            2,
            "image preparation owns its actual node definitions"
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        for (i, image) in images.iter().enumerate() {
            let inspection = image
                .inspection(ProcessingParameters::source(), None)
                .unwrap();
            image
                .prepare_calculation(&device, &mut encoder, &inspection, &Default::default())
                .unwrap();
            assert_eq!(
                live_calculations(),
                1 - i,
                "replacing each image releases its former arguments"
            );
        }
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        println!(
            "captured_calculation_argument_charge={} excludes=shaders_GPU_and_other_snapshot_storage",
            2 * bytes
        );
    });
}

#[test]
fn capturing_other_photos_does_not_retain_unreferenced_recipes() {
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
            let input = Arc::new(
                editor
                    .positive_recipe_for_source(
                        calibration,
                        editor.document.project.roll,
                        &source,
                        editor.photo_settings(),
                    )
                    .unwrap(),
            );
            tracked.push(Arc::downgrade(&input));
            if i == 35 {
                held = Some(input);
            }
        }
        assert_eq!(
            tracked
                .iter()
                .filter(|recipe| recipe.strong_count() != 0)
                .count(),
            1,
            "only the explicitly held photograph owns its capture"
        );
        drop(held);
        assert!(tracked.iter().all(|recipe| recipe.upgrade().is_none()));
    });
}

#[test]
fn document_replacement_keeps_active_calculations_and_device_infrastructure_not_hidden_recipes() {
    pollster::block_on(async {
        let (mut editor, _, _) = fixture().await;
        let recipe = Arc::new(
            editor
                .positive_recipe(
                    editor.input(),
                    editor.document.project.roll,
                    editor.photo_settings(),
                )
                .unwrap(),
        );
        let capture = Arc::downgrade(&recipe);
        let image = editor
            .document
            .visit
            .canvas
            .ready
            .as_ref()
            .unwrap()
            .proxy
            .image(editor.document.target())
            .unwrap();
        let calculation = image.recipe_calculation(&recipe, None).unwrap();
        let arguments = Arc::downgrade(&calculation);
        let captured = editor.document.capture();
        let id = editor.document.visit.current_source;
        editor.document = editor.document.replacement(captured);
        assert!(
            capture.upgrade().is_some(),
            "an active consumer owns its recipe independently"
        );
        assert!(editor.document.visit.working_view.device().is_some());
        drop(recipe);
        assert!(
            capture.upgrade().is_none(),
            "operations do not retain a second intent representation"
        );
        assert!(
            arguments.upgrade().is_some(),
            "the active calculation owns its arguments"
        );
        drop(calculation);
        assert!(arguments.upgrade().is_none());
        editor.document.visit.current_source = id;
        assert!(
            editor
                .positive_recipe(
                    editor.input(),
                    editor.document.project.roll,
                    editor.photo_settings()
                )
                .is_ok()
        );
    });
}
