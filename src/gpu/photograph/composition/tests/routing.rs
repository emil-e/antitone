use super::*;
use crate::gpu::photograph::request::PhotographRequest;
use crate::gpu::source::{ProcessingSource, SourceGeometry};
use crate::gpu::view::request::SourceRequest;
use crate::rendering::{OutputReference, OutputTarget};

#[derive(Debug, PartialEq)]
struct Suffix;
impl Suffix {
    fn compose(&self, input: Value) -> Result<Value, Error> {
        let meaning = input.domain().meaning;
        evaluation::value::pointwise(
            input,
            meaning,
            evaluation::value::Operation::new(
                "purpose routing suffix",
                (),
                "fn pixel(cell:vec2i,position:vec2f,a:vec4f)->vec4f { return vec4f(0.5*a.rgb*a.rgb+vec3f(0.08,-0.04,0.02),a.w); }",
                "pixel",
                evaluation::value::Access::Pointwise,
            ),
        )
    }
}

struct Pixels(Domain, Vec<[f32; 4]>);
impl evaluation::Source for Pixels {
    fn domain(&self) -> Domain {
        self.0
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        let bounds = self.0.finite()?;
        Ok((region.y..region.bottom())
            .flat_map(|y| (region.x..region.right()).map(move |x| self.1[bounds.index([x, y])]))
            .collect())
    }
}

fn pixels(
    renderer: &crate::renderer::GpuRenderer,
    queue: &wgpu::Queue,
    image: Value,
) -> Vec<[f32; 4]> {
    let domain = image.domain();
    let bounds = domain.finite().unwrap();
    let request = renderer
        .evaluation()
        .request(
            image,
            Region {
                bounds,
                grid: domain.grid,
            },
            Budget {
                tile: [64, 64],
                scratch_bytes: 32 << 20,
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
            |_, values| {
                pixels.extend_from_slice(values);
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    pixels
}

#[test]
fn one_central_effect_addition_reaches_complete_purpose_endpoints_without_changing_prefixes() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let file = crate::raw::fixture::bayer_dng_with_rgb([2949, 11796, 5898]);
    let scan = crate::scan::Scan::open(file.path(), None).unwrap();
    let mut project = Project {
        input: crate::processing::InputSettings::Digital,
        ..Default::default()
    };
    project.roll.exposure = 1.;
    project.output.grain.enabled = false;
    project.output.halation.enabled = false;
    let id = project
        .add_source(Source {
            id: SourceId(0),
            path: file.path().into(),
            development_mode: crate::project::DevelopmentMode::Unity,
            development: SourceDevelopment::from_metadata(
                &scan.source().metadata(),
                scan.source().white_balance(),
            )
            .unwrap(),
            orientation: Default::default(),
            crop: None,
            repairs: Default::default(),
            adjustment_layers: None,
        })
        .unwrap();
    let recipe = Arc::new(project.image_recipe(id).unwrap());
    let capture = PhotographRequest::Recipe(recipe.clone());
    let codes = evaluation::camera_source(&scan).unwrap();
    let limits = device.limits();
    let proxy = crate::scan::CameraProxy::from_scan(&scan).unwrap();
    let camera = ProcessingSource::new_continuous(
        &device,
        &proxy.pixels,
        proxy.matrix,
        Some(
            SourceGeometry::new(
                [scan.area.x, scan.area.y, scan.area.width, scan.area.height],
                proxy.dimensions(),
            )
            .unwrap(),
        ),
    )
    .unwrap();
    let mut frame = crate::frame_raster::FrameRaster::full(recipe.composition).unwrap();
    frame.output = proxy.dimensions();
    let thumb = SourceRequest::for_frame(frame)
        .unwrap()
        .with_sampling(crate::acquisition::Sampling::CameraCells {
            region: scan.area,
            size: proxy.dimensions(),
        })
        .unwrap();
    let mut view = frame;
    view.output = [32, 24];
    let spatial = SourceRequest::for_frame(view)
        .unwrap()
        .with_sampling(crate::acquisition::Sampling::CameraProxy {
            region: scan.area,
            size: proxy.dimensions(),
        })
        .unwrap();
    let original = RecipeGraph::originals(codes.clone(), None, &recipe).unwrap();
    let roll = original.roll().unwrap();
    let native = original.processed(&limits).unwrap();
    let physical = PhotographGraph::preview(&limits, &scan, None, &recipe, view, None)
        .unwrap()
        .0
        .processed;
    let sampled = capture
        .sampled_graph(&limits, &camera, None, thumb.clone())
        .unwrap()
        .processed;
    let navigation = capture
        .sampled_graph(&limits, &camera, None, spatial.clone())
        .unwrap()
        .processed;
    let resident = capture
        .resident_processed(&renderer, &camera, proxy.dimensions(), None)
        .unwrap();
    let target = OutputTarget::Srgb;
    let display = OutputReference::DisplayReferred(target);
    let authored = crate::gpu::photograph::authoring::with_suffix(
        |image| Suffix.compose(image),
        || {
            let graph = RecipeGraph::originals(codes, None, &recipe).unwrap();
            assert!(roll.same_definition(&graph.roll().unwrap()));
            vec![
                (
                    "scene export",
                    native.clone(),
                    graph
                        .output(&renderer, OutputReference::SceneReferred)
                        .unwrap(),
                    OutputReference::SceneReferred,
                ),
                (
                    "display export",
                    native,
                    graph.output(&renderer, display).unwrap(),
                    display,
                ),
                (
                    "working",
                    physical,
                    crate::gpu::view::rendering::photograph(
                        &renderer,
                        &scan,
                        None,
                        &capture,
                        SourceRequest::for_frame(view).unwrap(),
                        target,
                    )
                    .unwrap()
                    .definition,
                    display,
                ),
                (
                    "thumbnail",
                    sampled.clone(),
                    capture
                        .sampled_output(&renderer, &camera, None, thumb.clone(), target)
                        .unwrap()
                        .0,
                    display,
                ),
                (
                    "proxy",
                    navigation,
                    capture
                        .sampled_output(&renderer, &camera, None, spatial, target)
                        .unwrap()
                        .0,
                    display,
                ),
                (
                    "retained",
                    sampled.clone(),
                    capture
                        .retained_output(&renderer, &sampled, None, &thumb, target)
                        .unwrap()
                        .0,
                    display,
                ),
                (
                    "resident",
                    resident,
                    capture
                        .resident_output(&renderer, &camera, proxy.dimensions(), None, target)
                        .unwrap(),
                    display,
                ),
            ]
        },
    );
    // This checks routing and placement relative to grading, not the DRT's numerical accuracy.
    // Expected photographic input is the unmodified purpose's output; the added arithmetic is independent.
    for (name, baseline, actual, reference) in authored {
        let before = pixels(&renderer, &queue, baseline.clone());
        let changed = before
            .iter()
            .map(|p| {
                [
                    0.5 * p[0] * p[0] + 0.08,
                    0.5 * p[1] * p[1] - 0.04,
                    0.5 * p[2] * p[2] + 0.02,
                    p[3],
                ]
            })
            .collect();
        let expected = compose_output(
            &renderer,
            evaluation::source(Arc::new(Pixels(baseline.domain(), changed))),
            reference,
            &Default::default(),
        )
        .unwrap();
        let plain = compose_output(&renderer, baseline, reference, &Default::default()).unwrap();
        let expected = pixels(&renderer, &queue, expected);
        let plain = pixels(&renderer, &queue, plain);
        let actual = pixels(&renderer, &queue, actual);
        assert_eq!(actual.len(), expected.len(), "{name}");
        assert!(
            expected.iter().zip(&plain).any(|(a, b)| a[..3]
                .iter()
                .zip(&b[..3])
                .any(|(a, b)| (a - b).abs() > 1e-3)),
            "{name}: contrasting effect control"
        );
        for (a, b) in actual.iter().zip(&expected) {
            assert!(
                a.iter().zip(b).all(|(a, b)| (a - b).abs() < 3e-5),
                "{name}: {a:?} != {b:?}"
            );
        }
    }
}

#[test]
fn returned_recipe_requests_replan_with_the_captured_photograph_after_preview_sampling_and_regrading()
 {
    use crate::acquisition::Sampling;
    let (device, _, renderer) = crate::gpu::test_support::render_context();
    let limits = device.limits();
    let file = crate::raw::fixture::bayer_dng_with_rgb([2949, 11796, 5898]);
    let scan = crate::scan::Scan::open(file.path(), None).unwrap();
    let mut project = Project {
        input: crate::processing::InputSettings::Digital,
        ..Default::default()
    };
    let id = project
        .add_source(Source {
            id: SourceId(0),
            path: file.path().into(),
            development_mode: crate::project::DevelopmentMode::Unity,
            development: SourceDevelopment::from_metadata(
                &scan.source().metadata(),
                scan.source().white_balance(),
            )
            .unwrap(),
            orientation: Default::default(),
            crop: None,
            repairs: Default::default(),
            adjustment_layers: None,
        })
        .unwrap();
    project.output.grain.enabled = true;
    project.output.grain.amount = 0.;
    project.output.halation.enabled = true;
    let recipe = Arc::new(project.image_recipe(id).unwrap());
    let capture = PhotographRequest::Recipe(recipe.clone());
    let mut frame = crate::frame_raster::FrameRaster::full(recipe.composition).unwrap();
    frame.output = [8, 6];
    let fresh = capture.source_request(frame).unwrap();
    assert!(
        fresh.plan().output().output[0] > frame.output[0],
        "the fixture must need photographic refinement"
    );

    let check = |capture: &PhotographRequest, returned: &SourceRequest| {
        let mut moved = returned.frame();
        moved.output = [7, 5];
        let proxy = Sampling::CameraProxy {
            region: scan.area,
            size: [scan.area.width, scan.area.height],
        };
        for sampling in [Sampling::NeutralFootprints, proxy] {
            let actual = returned
                .clone()
                .with_sampling(sampling)
                .unwrap()
                .with_frame(moved)
                .unwrap();
            let expected = capture
                .source_request(moved)
                .unwrap()
                .with_sampling(sampling)
                .unwrap();
            assert!(
                actual.plan().same_acquisition_layout(expected.plan()),
                "replanning must retain the current photograph's refinement and neighbours"
            );
        }
        let actual = returned.underlay().unwrap();
        let expected = capture
            .source_request(returned.frame())
            .unwrap()
            .with_sampling(returned.plan().sampling())
            .unwrap()
            .underlay()
            .unwrap();
        assert!(actual.plan().same_acquisition_layout(expected.plan()));
    };
    let (_, preview) = capture
        .preview_output(
            &renderer,
            &scan,
            None,
            SourceRequest::for_frame(frame).unwrap(),
            OutputTarget::Srgb,
        )
        .unwrap();
    check(&capture, &preview);

    let proxy = crate::scan::CameraProxy::from_scan(&scan).unwrap();
    let camera = ProcessingSource::new_continuous(
        &device,
        &proxy.pixels,
        proxy.matrix,
        Some(
            SourceGeometry::new(
                [scan.area.x, scan.area.y, scan.area.width, scan.area.height],
                proxy.dimensions(),
            )
            .unwrap(),
        ),
    )
    .unwrap();
    let mut full = frame;
    full.output = proxy.dimensions();
    let sampling = Sampling::CameraCells {
        region: scan.area,
        size: proxy.dimensions(),
    };
    let sampled = capture
        .sampled_graph(
            &limits,
            &camera,
            None,
            SourceRequest::for_frame(full)
                .unwrap()
                .with_sampling(sampling)
                .unwrap(),
        )
        .unwrap();
    check(&capture, &sampled.request);

    let acquired = capture
        .source_request(full)
        .unwrap()
        .with_sampling(sampling)
        .unwrap();
    let mut changed = (*recipe).clone();
    changed.grain.enabled = false;
    changed.halation.enabled = false;
    let changed = PhotographRequest::Recipe(Arc::new(changed));
    let retained = changed
        .retained_graph(&limits, &sampled.processed, None, &acquired)
        .unwrap();
    check(&changed, &retained.request);
}
