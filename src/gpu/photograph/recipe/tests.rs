use super::*;
use crate::gpu::evaluation;
use crate::gpu::interpretation::InspectionView;
use crate::project::{Development, Project, Source, SourceDevelopment, SourceId};
use glam::{DMat3, DVec3};

#[cfg(feature = "editor")]
mod reuse;

#[derive(Clone, Debug, PartialEq)]
struct CameraCodes;
impl evaluation::Source for CameraCodes {
    fn identity(&self) -> Option<evaluation::RetainedKey> {
        Some(evaluation::RetainedKey::new(self.clone()))
    }

    fn domain(&self) -> Domain {
        Domain {
            bounds: Some(CellRegion::new([0, 0], [3, 1])),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::CameraCodes,
        }
    }

    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        Ok(vec![
            [16383.75, 32767.5, 49151.25, 0.];
            region.width as usize
        ])
    }
}

fn project(input: crate::processing::InputSettings) -> (Project, SourceId) {
    let mut project = Project {
        input,
        ..Default::default()
    };
    let id = project
        .add_source(Source {
            development_mode: crate::project::DevelopmentMode::Unity,
            id: SourceId(0),
            path: "graph-input.dng".into(),
            development: SourceDevelopment {
                decode: Development::new("fixture".into(), [1.; 4]),
                dimensions: [3, 1],
                active_area: [0, 0, 3, 1],
                camera_model: "fixture".into(),
                camera_to_ap1: DMat3::from_diagonal(DVec3::new(1.2, 0.8, 0.4)).to_cols_array_2d(),
            },
            orientation: Default::default(),
            crop: None,
            repairs: Default::default(),
            adjustment_layers: None,
        })
        .unwrap();
    (project, id)
}

fn recipe() -> ImageRecipe {
    let (project, id) = project(Default::default());
    project.image_recipe(id).unwrap()
}

fn read(
    renderer: &crate::renderer::GpuRenderer,
    queue: &wgpu::Queue,
    image: Arc<ImageNode>,
) -> Vec<[f32; 4]> {
    let domain = image.domain();
    let request = renderer
        .evaluation()
        .request(
            image,
            Region {
                bounds: domain.finite().unwrap(),
                grid: domain.grid,
            },
            Budget {
                tile: [3, 1],
                scratch_bytes: 1 << 20,
            },
        )
        .unwrap();
    let mut pixels = Vec::new();
    renderer
        .evaluation()
        .read(
            queue,
            &Default::default(),
            &request,
            || false,
            |_, block| {
                pixels.extend_from_slice(block);
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    pixels
}

struct PatternCodes(crate::geometry::SourceRegion);

impl evaluation::Source for PatternCodes {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some(self.0.try_into().unwrap()),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::CameraCodes,
        }
    }

    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        Ok((region.y..region.bottom())
            .flat_map(|y| {
                (region.x..region.right()).map(move |x| {
                    if (x / 11 + y / 7) % 2 == 0 {
                        [12000., 4000., 23000., 0.]
                    } else {
                        [75000., 42000., 9000., 0.]
                    }
                })
            })
            .collect())
    }
}

fn near(pixels: &[[f32; 4]], expected: DVec3) {
    assert_eq!(pixels.len(), 3);
    for pixel in pixels {
        let actual = DVec3::new(pixel[0].into(), pixel[1].into(), pixel[2].into());
        assert!(
            (actual - expected).abs().max_element() < 2e-6,
            "{actual:?} != {expected:?}"
        );
    }
}

#[test]
fn an_uncalibrated_negative_has_camera_and_inspection_endpoints_but_no_positive() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let recipe = recipe();
    assert!(!recipe.neutral.input.ready());
    let source = evaluation::source(Arc::new(CameraCodes));
    let graph = RecipeGraph::originals(source, None, &recipe, &Default::default()).unwrap();
    near(
        &read(&renderer, &queue, graph.source.camera.clone()),
        DVec3::new(0.25, 0.5, 0.75),
    );
    let inspection = graph
        .source
        .inspection(
            InspectionView::new(DMat3::from_diagonal(DVec3::new(0.5, 1.1, 1.4)), 1.).unwrap(),
        )
        .unwrap();
    assert_eq!(inspection.domain().meaning, Meaning::Inspection);
    // Normalization, camera-to-working conversion, then this viewer's WB/exposure.
    near(
        &read(&renderer, &queue, inspection),
        DVec3::new(0.3, 0.88, 0.84),
    );
    assert!(
        graph.interpreted().is_err(),
        "positive interpretation needs calibration"
    );
    assert!(
        graph.photograph(&device.limits()).is_err(),
        "the complete positive needs calibration"
    );
}

#[test]
fn correction_is_shared_upstream_of_positive_and_inspection_but_recorded_exposure_is_not() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let (project, id) = project(crate::processing::InputSettings::Digital);
    let correction = Arc::new(
        crate::flat_field::FlatField::from_nodes(
            [0, 0, 3, 1],
            [2, 2],
            vec![DVec3::new(2., 0.5, 1.25); 4],
        )
        .unwrap(),
    );
    let recipe = project
        .capture_image(project.source(id).unwrap(), |recipe| {
            recipe.neutral.reference = Some(crate::exposure_reference::ReferenceExposure::Matched(
                crate::exposure_reference::StartingExposure {
                    ev: 1.,
                    samples: 128,
                    spread: 0.,
                },
            ));
            recipe.neutral.correction = Some(correction.clone());
        })
        .unwrap();
    let field = Arc::new(GpuFlatField::new(&device, correction).unwrap());
    let source = evaluation::source(Arc::new(CameraCodes));
    let graph =
        RecipeGraph::originals(source.clone(), Some(field), &recipe, &Default::default()).unwrap();
    let camera = graph.source.camera.clone();
    let positive = graph.interpreted().unwrap().unrepaired;
    let inspection = graph
        .source
        .inspection(InspectionView::new(DMat3::IDENTITY, 0.).unwrap())
        .unwrap();
    near(
        &read(&renderer, &queue, camera.clone()),
        DVec3::new(0.5, 0.25, 0.9375),
    );
    near(
        &read(&renderer, &queue, positive.clone()),
        DVec3::new(1.2, 0.4, 0.75),
    );
    near(
        &read(&renderer, &queue, inspection.clone()),
        DVec3::new(0.6, 0.2, 0.375),
    );
    let grading = Grading::new(
        &device.limits(),
        recipe.roll,
        recipe.grade.clone(),
        [0, 0, 3, 1].into(),
    )
    .unwrap();
    assert!(grading.roll(positive.clone()).is_ok());
    assert!(
        grading.roll(inspection.clone()).is_err(),
        "source-view light cannot substitute for the positive prefix"
    );
    #[cfg(feature = "editor")]
    {
        let spec = crate::gpu::view::request::SourceRequest::for_recipe(
            crate::frame_raster::FrameRaster::full(recipe.composition).unwrap(),
            &recipe,
        )
        .unwrap();
        assert!(
            PhotographGraph::filtered_recipe(&device.limits(), &spec, positive.clone(), &recipe)
                .is_ok()
        );
        assert!(
            PhotographGraph::filtered_recipe(&device.limits(), &spec, inspection.clone(), &recipe)
                .is_err(),
            "an inspection of the same corrected camera cells cannot satisfy photographic filtering"
        );
    }
    for branch in [positive, inspection] {
        let mut upstream = branch;
        while upstream.domain().meaning != Meaning::Camera {
            upstream = upstream
                .input()
                .expect("branch retains corrected camera input")
                .clone();
        }
        assert!(
            upstream.same_definition(&camera),
            "view choices cannot redefine camera correction"
        );
    }
    let missing_field = RecipeGraph::originals(source, None, &recipe, &Default::default());
    assert!(
        missing_field.is_err(),
        "an accepted correction cannot silently disappear"
    );
}

#[cfg(feature = "editor")]
#[test]
fn recipe_quality_preserves_the_direct_operations_and_pixels_for_original_and_physical_views() {
    use crate::adjustment_layers::AdjustmentLayerSettings;
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let (mut project, id) = project(crate::processing::InputSettings::Digital);
    let source = project.source_mut(id).unwrap();
    source.development.dimensions = [1000, 800];
    source.development.active_area = [0, 0, 1000, 800];
    source.adjustment_layers = Some(crate::project::AdjustmentLayers::with_global(
        AdjustmentLayerSettings {
            exposure: -0.2,
            contrast: 1.15,
            saturation: 0.9,
            ..Default::default()
        },
    ));
    project.roll.exposure = 0.3;
    project.output.grain = crate::grain::GrainSettings {
        enabled: true,
        size: 0.1,
        image_sigma: 0.1,
        amount: 0.13,
        colour: 0.65,
    };
    project.output.halation.enabled = true;
    let recipe = project.image_recipe(id).unwrap();
    assert!(
        recipe
            .grain
            .parameters(recipe.composition.crop().size)
            .unwrap()
            .is_some()
    );
    assert!(
        recipe
            .halation
            .parameters(recipe.composition.crop().size)
            .unwrap()
            .is_some()
    );
    let codes = evaluation::source(Arc::new(PatternCodes([0, 0, 1000, 800].into())));
    let direct = AuthoredPhotograph::from_recipe(&device.limits(), &recipe).unwrap();
    let captured =
        RecipeGraph::originals(codes.clone(), None, &recipe, &Default::default()).unwrap();
    let original = captured.photograph(&device.limits()).unwrap();
    let original_direct = PhotographGraph::new_with_stages(
        &device.limits(),
        codes.clone(),
        None,
        &direct,
        direct.source_transform().unwrap().matrix,
        &direct.pixel_pipeline(),
    )
    .unwrap();
    // Migration consistency with the direct-operation API, not a second photographic oracle.
    assert!(
        original
            .processed
            .same_definition(&original_direct.processed)
    );
    let probe = CellRegion::new([395, 300], [3, 1]);
    let original_pixels = read(
        &renderer,
        &queue,
        window(original.processed, probe).unwrap(),
    );
    assert_eq!(
        original_pixels,
        read(
            &renderer,
            &queue,
            window(original_direct.processed, probe).unwrap()
        )
    );
    let mut clean = project.clone();
    clean.output.grain.enabled = false;
    clean.output.halation.enabled = false;
    let clean = clean.image_recipe(id).unwrap();
    let clean = RecipeGraph::originals(codes.clone(), None, &clean, &Default::default())
        .unwrap()
        .photograph(&device.limits())
        .unwrap();
    let clean_pixels = read(&renderer, &queue, window(clean.processed, probe).unwrap());
    assert!(
        original_pixels
            .iter()
            .zip(clean_pixels)
            .any(|(a, b)| (0..3).any(|channel| (a[channel] - b[channel]).abs() > 1e-4)),
        "control: the fixture must distinguish active texture from a clean plate"
    );

    let mut frame = crate::frame_raster::FrameRaster::full(recipe.composition).unwrap();
    frame.output = [29, 23];
    let (view, plan) = captured
        .physical_view(&device.limits(), frame, None)
        .unwrap();
    let (view_direct, direct_plan) =
        PhotographGraph::preview_with_input(&device.limits(), codes, None, &direct, frame, None)
            .unwrap();
    assert_eq!(plan.output(), direct_plan.output());
    assert_eq!(plan.input(), direct_plan.input());
    assert!(view.processed.same_definition(&view_direct.processed));
    let probe = CellRegion::new([12, 9], [3, 1]);
    assert_eq!(
        read(&renderer, &queue, window(view.processed, probe).unwrap()),
        read(
            &renderer,
            &queue,
            window(view_direct.processed, probe).unwrap()
        )
    );
}

#[cfg(feature = "editor")]
#[test]
fn proxy_quality_omits_repair_execution_without_stripping_the_captured_intent() {
    use crate::{
        dust_repair::{RepairOrigin, SupportMask},
        gpu::source::SourceGeometry,
    };
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let (mut project, id) = project(crate::processing::InputSettings::Digital);
    let source = project.source_mut(id).unwrap();
    source.development.dimensions = [64, 48];
    source.development.active_area = [0, 0, 64, 48];
    source
        .repairs
        .insert(
            SupportMask::new([16, 16], [2, 2], vec![true; 4]).unwrap(),
            RepairOrigin::Manual,
            [24, 0],
        )
        .unwrap();
    let recipe = project.image_recipe(id).unwrap();
    let area = recipe.composition.area();
    assert_eq!(recipe.repairs.len(), 1);
    let repaired = [-0.125, 2., 0.25];
    let evaluated =
        crate::dust_repair::evaluate(&recipe.repairs[0], area, |_| Ok(repaired)).unwrap();
    let repairs = crate::gpu::repairs::Snapshot::logical(vec![Arc::new(evaluated)]).unwrap();
    let codes = evaluation::source(Arc::new(PatternCodes(area)));
    let original = RecipeGraph::originals(codes, None, &recipe, &repairs)
        .unwrap()
        .photograph(&device.limits())
        .unwrap();
    let patch = read(
        &renderer,
        &queue,
        window(original.neutral, CellRegion::new([16, 16], [1, 1])).unwrap(),
    );
    assert_eq!(
        &patch[0][..3],
        &repaired,
        "control: the supplied repair must change original working pixels"
    );

    let matrix = DMat3::from_cols_array_2d(
        &recipe
            .neutral
            .source_development
            .resolved()
            .unwrap()
            .camera_to_ap1,
    );
    let camera = ProcessingSource::new_continuous(
        &device,
        &vec![[16383.75, 32767.5, 49151.25, 0.]; 32 * 24],
        matrix,
        Some(SourceGeometry::new([area.x, area.y, area.width, area.height], [32, 24]).unwrap()),
    )
    .unwrap();
    let mut frame = crate::frame_raster::FrameRaster::full(recipe.composition).unwrap();
    frame.output = [32, 24];
    let spec = crate::gpu::view::request::SourceRequest::for_recipe(frame, &recipe)
        .unwrap()
        .with_sampling(crate::acquisition::Sampling::CameraCells {
            region: area,
            size: [32, 24],
        })
        .unwrap()
        .with_repairs(repairs);
    let request = super::super::request::PhotographRequest::Recipe(Arc::new(recipe.clone()));
    let graph = request
        .sampled_graph(&device.limits(), &camera, None, spec)
        .unwrap();
    assert!(graph.request.repairs.0.is_empty());
    assert_eq!(request.recipe().unwrap().repairs, recipe.repairs);
    let pixels = read(
        &renderer,
        &queue,
        window(graph.processed.clone(), CellRegion::new([8, 8], [3, 1])).unwrap(),
    );
    for pixel in pixels {
        let actual = DVec3::from_array([pixel[0].into(), pixel[1].into(), pixel[2].into()]);
        assert!((actual - DVec3::new(0.3, 0.4, 0.3)).abs().max_element() < 2e-6);
    }
    project.source_mut(id).unwrap().repairs.clear();
    let clean = project.image_recipe(id).unwrap();
    assert_ne!(recipe, clean);
    let clean_request = super::super::request::PhotographRequest::Recipe(Arc::new(clean));
    let clean = clean_request
        .sampled_graph(&device.limits(), &camera, None, graph.request.clone())
        .unwrap();
    assert!(graph.baseline.same_definition(&clean.baseline));
    assert!(graph.processed.same_definition(&clean.processed));
    let scope = renderer.evaluation().test_materializations(false);
    let bounds = CellRegion::new([8, 8], [3, 1]);
    let repaired_proxy = reuse::materialize(&renderer, &queue, &scope, graph.processed, bounds);
    let clean_proxy = reuse::materialize(&renderer, &queue, &scope, clean.processed, bounds);
    assert!(Arc::ptr_eq(&repaired_proxy.image, &clean_proxy.image));
    let source_lifetime = Arc::downgrade(&camera.identity);
    drop(camera);
    assert!(
        source_lifetime.upgrade().is_none(),
        "cached keys do not own the uploaded source"
    );

    let changed_camera = ProcessingSource::new_continuous(
        &device,
        &vec![[32767.5, 16383.75, 8191.875, 0.]; 32 * 24],
        matrix,
        Some(SourceGeometry::new([area.x, area.y, area.width, area.height], [32, 24]).unwrap()),
    )
    .unwrap();
    let changed = clean_request
        .sampled_graph(&device.limits(), &changed_camera, None, graph.request)
        .unwrap();
    let changed = reuse::materialize(&renderer, &queue, &scope, changed.processed, bounds);
    assert!(!Arc::ptr_eq(&repaired_proxy.image, &changed.image));
    for pixel in evaluation::read_buffer(&device, &queue, changed.as_buffer().unwrap()) {
        let actual = DVec3::from_array([pixel[0].into(), pixel[1].into(), pixel[2].into()]);
        assert!((actual - DVec3::new(0.6, 0.2, 0.05)).abs().max_element() < 2e-6);
    }
}

#[cfg(feature = "editor")]
#[test]
fn filtered_recipe_keeps_the_photographs_mask_extent_instead_of_redefining_it_from_demand() {
    use crate::adjustment_layers::{
        AdjustmentLayer, AdjustmentLayerCoverage, AdjustmentLayerSettings,
    };
    use crate::geometry::{Composition, SourcePoint};
    use crate::gpu::view::request::SourceRequest;

    let (device, _queue, _renderer) = crate::gpu::test_support::render_context();
    let (mut project, id) = project(crate::processing::InputSettings::Digital);
    let mut layer = AdjustmentLayer::global(AdjustmentLayerSettings {
        exposure: 1.,
        ..Default::default()
    });
    layer.mask = AdjustmentLayerCoverage::Radial {
        center: SourcePoint([1.5, 0.5]),
        radii: [1., 1.],
        angle: 0.,
        feather: 0.,
    };
    let mut layers = crate::project::AdjustmentLayers::default();
    layers.add("Radial".into(), layer).unwrap();
    project.source_mut(id).unwrap().adjustment_layers = Some(layers);
    let recipe = project.image_recipe(id).unwrap();
    let codes = evaluation::source(Arc::new(CameraCodes));
    let neutral = RecipeGraph::originals(codes, None, &recipe, &Default::default())
        .unwrap()
        .interpreted()
        .unwrap()
        .unrepaired;
    let full = SourceRequest::for_recipe(
        crate::frame_raster::FrameRaster::full(recipe.composition).unwrap(),
        &recipe,
    )
    .unwrap();
    assert!(
        PhotographGraph::filtered_recipe(&device.limits(), &full, neutral.clone(), &recipe).is_ok()
    );
    let other_extent = SourceRequest::for_frame(
        crate::frame_raster::FrameRaster::full(
            Composition::new([1, 0, 1, 1].into(), Default::default(), Default::default()).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(
        PhotographGraph::filtered_recipe(&device.limits(), &other_extent, neutral, &recipe)
            .is_err()
    );
}
