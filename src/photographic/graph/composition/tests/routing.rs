use super::*;
use crate::acquisition::camera::{ProcessingSource, SourceGeometry};
use crate::photographic::graph::request::PhotographRequest;
use crate::photographic::graph::view_source::SourceRequest;
use crate::rendering::OutputTarget;

#[derive(Debug, PartialEq)]
struct Suffix;
impl Suffix {
    fn compose(&self, input: Value) -> Result<Value, Error> {
        evaluation::value::pointwise(
            input,
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
    renderer: &crate::render_fixture::RenderFixture,
    queue: &crate::gpu::submission::Queue,
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
            Tiling { tile: [64, 64] },
        )
        .unwrap();
    let mut pixels = Vec::new();
    renderer
        .evaluation()
        .read(
            queue,
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
    let (device, queue, renderer) = crate::photographic::test_support::render_context();
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
            capture_time: None,
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
    let codes = crate::acquisition::camera::camera_source(&scan).unwrap();
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
    let roll = || Grading::roll_image(original_neutral(codes.clone(), None, &recipe)?, recipe.roll);
    let roll_before = roll().unwrap();
    let native = processed_originals(&limits, codes.clone(), None, &recipe).unwrap();
    let (physical, _) = preview(&limits, &scan, None, &recipe, view).unwrap();
    let (sampled, _) = capture
        .sampled_view(&limits, &camera, None, thumb.clone())
        .unwrap();
    let (navigation, _) = capture
        .sampled_view(&limits, &camera, None, spatial.clone())
        .unwrap();
    let resident = capture
        .resident_processed(&device.limits(), &camera, proxy.dimensions(), None)
        .unwrap();
    let target = OutputTarget::Srgb;
    let display = Some(target);
    let authored = crate::photographic::graph::authoring::with_suffix(
        |image| Suffix.compose(image),
        || {
            assert!(roll_before.same_definition(&roll().unwrap()));
            let processed = processed_originals(&limits, codes.clone(), None, &recipe).unwrap();
            vec![
                ("scene export", native.clone(), processed.clone(), None),
                (
                    "display export",
                    native,
                    display_output(
                        processed,
                        &recipe.luts,
                        &renderer.looks,
                        renderer.drt.get(target),
                    )
                    .unwrap(),
                    display,
                ),
                (
                    "working",
                    physical,
                    crate::viewer::view::rendering::photograph(
                        &limits,
                        &scan,
                        None,
                        &capture,
                        SourceRequest::for_frame(view).unwrap(),
                        renderer.looks(),
                        renderer.drt(target),
                    )
                    .unwrap()
                    .definition,
                    display,
                ),
                (
                    "thumbnail",
                    sampled.clone(),
                    capture
                        .sampled_output(
                            &device.limits(),
                            &camera,
                            None,
                            thumb.clone(),
                            renderer.looks(),
                            renderer.drt(target),
                        )
                        .unwrap()
                        .0,
                    display,
                ),
                (
                    "proxy",
                    navigation,
                    capture
                        .sampled_output(
                            &device.limits(),
                            &camera,
                            None,
                            spatial,
                            renderer.looks(),
                            renderer.drt(target),
                        )
                        .unwrap()
                        .0,
                    display,
                ),
                (
                    "retained",
                    sampled.clone(),
                    capture
                        .retained_output(
                            &device.limits(),
                            &sampled,
                            None,
                            &thumb,
                            renderer.looks(),
                            renderer.drt(target),
                        )
                        .unwrap()
                        .0,
                    display,
                ),
                (
                    "resident",
                    resident,
                    capture
                        .resident_output(
                            &device.limits(),
                            &camera,
                            proxy.dimensions(),
                            None,
                            renderer.looks(),
                            renderer.drt(target),
                        )
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
        let expected = evaluation::source(Arc::new(Pixels(baseline.domain(), changed)));
        let (expected, plain) = match reference {
            Some(target) => {
                let drt = renderer.drt.get(target);
                (drt.apply(expected).unwrap(), drt.apply(baseline).unwrap())
            }
            None => (expected, baseline),
        };
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
fn preview_and_proxy_layouts_can_be_replanned_with_an_explicit_capture() {
    use crate::acquisition::Sampling;
    let (device, _, renderer) = crate::photographic::test_support::render_context();
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
            capture_time: None,
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
    let identity = crate::acquisition::camera::identity::Identity::camera(scan.source().identity());
    let fresh = capture
        .source_request(frame)
        .unwrap()
        .with_identity(identity.clone());
    assert!(
        fresh.plan().output().output[0] > frame.output[0],
        "the fixture must need photographic refinement"
    );

    let mut changed = (*recipe).clone();
    changed.grain.enabled = false;
    changed.halation.enabled = false;
    let changed = PhotographRequest::Recipe(Arc::new(changed));
    let check = |returned: &SourceRequest| {
        let moved = crate::frame_raster::FrameRaster {
            output: [7, 5],
            ..returned.frame()
        };
        let proxy = Sampling::CameraProxy {
            region: scan.area,
            size: [scan.area.width, scan.area.height],
        };
        for frame in [
            moved,
            crate::frame_raster::FrameRaster::full(moved.composition).unwrap(),
        ] {
            // Navigation chooses the frame; the current capture determines its working grid.
            let demand = SourceRequest::for_frame(frame)
                .unwrap()
                .with_identity(returned.identity.clone());
            for sampling in [Sampling::NeutralFootprints, proxy] {
                let refined = capture.plan_sampling(demand.clone(), sampling).unwrap();
                let plain = changed.plan_sampling(demand.clone(), sampling).unwrap();
                for planned in [&refined, &plain] {
                    assert_eq!(planned.frame(), frame);
                    assert_eq!(planned.plan().sampling(), sampling);
                    assert!(planned.same_source(returned));
                }
                assert_eq!(plain.plan().output(), frame);
                if frame == moved {
                    assert!(refined.plan().output().output[0] > frame.output[0]);
                }
            }
        }
    };
    let (_, preview) = capture
        .preview_output(
            &device.limits(),
            &scan,
            None,
            SourceRequest::for_frame(frame)
                .unwrap()
                .with_identity(identity.clone()),
            renderer.looks(),
            renderer.drt(OutputTarget::Srgb),
        )
        .unwrap();
    check(&preview);

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
    let (sampled, sampled_spec) = capture
        .sampled_view(
            &limits,
            &camera,
            None,
            SourceRequest::for_frame(full)
                .unwrap()
                .with_identity(identity.clone())
                .with_sampling(sampling)
                .unwrap(),
        )
        .unwrap();
    check(&sampled_spec);

    let acquired = capture
        .plan_sampling(
            capture
                .source_request(full)
                .unwrap()
                .with_identity(identity),
            sampling,
        )
        .unwrap();
    let (_, retained) = changed
        .retained_view(&limits, &sampled, None, &acquired)
        .unwrap();
    assert!(retained.same_source(&sampled_spec));
    assert_eq!(retained.frame(), full);
    assert!(retained.plan().same_acquisition_layout(acquired.plan()));
    check(&retained);
}
