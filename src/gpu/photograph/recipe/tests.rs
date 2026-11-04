use super::*;
use crate::gpu::evaluation;
use crate::gpu::interpretation::InspectionView;
use crate::project::{Development, Project, Source, SourceDevelopment, SourceId};
use glam::{DMat3, DVec3};

struct CameraCodes;
impl evaluation::Source for CameraCodes {
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

fn recipe() -> ImageRecipe {
    let mut project = Project::default();
    let id = project
        .add_source(Source {
            id: SourceId(0),
            path: "graph-input.dng".into(),
            reference_only: false,
            development: SourceDevelopment {
                decode: Development::new("fixture".into(), [1.; 4]),
                lens_correction: None,
                dng_development: None,
                dimensions: [3, 1],
                active_area: [0, 0, 3, 1],
                camera_model: "fixture".into(),
                camera_to_ap1: DMat3::from_diagonal(DVec3::new(1.2, 0.8, 0.4)).to_cols_array_2d(),
                input_floor: crate::processing::CAMERA_INPUT_FLOOR,
            },
            orientation: Default::default(),
            crop: None,
            repairs: Default::default(),
            adjustment_layers: None,
        })
        .unwrap();
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
    let endpoint = |at| {
        PhotographGraph::recipe_at(
            &device.limits(),
            source.clone(),
            None,
            &recipe,
            &Default::default(),
            at,
        )
    };
    let camera = endpoint(Boundary::Camera).unwrap();
    near(
        &read(&renderer, &queue, camera),
        DVec3::new(0.25, 0.5, 0.75),
    );
    let inspection = endpoint(Boundary::Inspection(
        InspectionView::new(DMat3::from_diagonal(DVec3::new(0.5, 1.1, 1.4)), 1.).unwrap(),
    ))
    .unwrap();
    assert_eq!(inspection.domain().meaning, Meaning::Inspection);
    // Normalization, camera-to-working conversion, then this viewer's WB/exposure.
    near(
        &read(&renderer, &queue, inspection),
        DVec3::new(0.3, 0.88, 0.84),
    );
    for at in [
        Boundary::Native,
        Boundary::Unrepaired,
        Boundary::Neutral,
        Boundary::Roll,
        Boundary::Graded,
        Boundary::Processed,
    ] {
        assert!(
            endpoint(at).is_err(),
            "an inversion endpoint needs calibration"
        );
    }
    assert!(
        PhotographGraph::from_recipe(&device.limits(), source, None, &recipe, &Default::default(),)
            .is_err()
    );
}

#[test]
fn correction_is_shared_upstream_of_positive_and_inspection_but_recorded_exposure_is_not() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let mut recipe = recipe();
    recipe.neutral.input = crate::processing::InputSettings::Digital;
    recipe.neutral.reference = Some(crate::exposure_reference::ReferenceExposure::Matched(
        crate::exposure_reference::StartingExposure {
            ev: 1.,
            samples: 128,
            spread: 0.,
        },
    ));
    let correction = Arc::new(
        crate::flat_field::FlatField::from_nodes(
            [0, 0, 3, 1],
            [2, 2],
            vec![DVec3::new(2., 0.5, 1.25); 4],
        )
        .unwrap(),
    );
    recipe.neutral.correction = Some(correction.clone());
    let field = Arc::new(GpuFlatField::new(&device, correction).unwrap());
    let source = evaluation::source(Arc::new(CameraCodes));
    let endpoint = |at| {
        PhotographGraph::recipe_at(
            &device.limits(),
            source.clone(),
            Some(field.clone()),
            &recipe,
            &Default::default(),
            at,
        )
        .unwrap()
    };
    let camera = endpoint(Boundary::Camera);
    let positive = endpoint(Boundary::Unrepaired);
    let inspection = endpoint(Boundary::Inspection(
        InspectionView::new(DMat3::IDENTITY, 0.).unwrap(),
    ));
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
    let missing_field = PhotographGraph::recipe_at(
        &device.limits(),
        source,
        None,
        &recipe,
        &Default::default(),
        Boundary::Camera,
    );
    assert!(
        missing_field.is_err(),
        "an accepted correction cannot silently disappear"
    );
}

#[cfg(feature = "editor")]
#[test]
fn filtered_recipe_keeps_the_photographs_mask_extent_instead_of_redefining_it_from_demand() {
    use crate::adjustment_layers::{
        AdjustmentLayer, AdjustmentLayerCoverage, AdjustmentLayerSettings, AdjustmentLayerSnapshot,
    };
    use crate::geometry::{Composition, SourcePoint};
    use crate::gpu::view::request::SourceRequest;

    let (device, _queue, _renderer) = crate::gpu::test_support::render_context();
    let mut recipe = recipe();
    recipe.neutral.input = crate::processing::InputSettings::Digital;
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
    recipe.grade = AdjustmentLayerSnapshot::new(vec![layer]).unwrap();
    let codes = evaluation::source(Arc::new(CameraCodes));
    let neutral = PhotographGraph::recipe_at(
        &device.limits(),
        codes,
        None,
        &recipe,
        &Default::default(),
        Boundary::Unrepaired,
    )
    .unwrap();
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
