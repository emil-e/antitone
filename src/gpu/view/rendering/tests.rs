use super::*;
use crate::gpu::evaluation::numeric_fixture::NumericKey;

mod failures;
use crate::gpu::evaluation as e;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Coordinates(Arc<AtomicUsize>);
impl e::Source for Coordinates {
    fn domain(&self) -> e::Domain {
        e::Domain {
            bounds: Some([0, 0, 8, 4].into()),
            grid: e::Grid::axis_aligned([0.; 2], [1.; 2]),
        }
    }

    fn identity(&self) -> Option<e::RetainedKey> {
        Some(e::RetainedKey::new("described view coordinates"))
    }

    fn read(&self, region: e::CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok((region.y..region.bottom())
            .flat_map(|y| (region.x..region.right()).map(move |x| [x as f32, y as f32, -2., 8.]))
            .collect())
    }
}

fn endpoint(input: e::Value, delta: f32) -> e::Value {
    e::value::pointwise(
        input,
        e::value::Operation::new(
            "selected view stage", NumericKey(delta),
            "struct Parameters { delta:vec4f } fn pixel(p:vec2i,position:vec2f,a:vec4f,v:Parameters)->vec4f { return vec4f(a.rgb+v.delta.xyz,a.w); }",
            "pixel", e::value::Access::Pointwise,
        ).with_uniform("Parameters", &[delta, 0., 0., 0.]),
    ).unwrap()
}

#[cfg(feature = "ui-tests")]
#[test]
fn source_reuse_observation_rejects_independently_recomputed_equal_pixels() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let kernels = pollster::block_on(ViewKernels::new(&device)).unwrap();
    let reads = Arc::new(AtomicUsize::new(0));
    let input = e::source(Arc::new(Coordinates(reads.clone())));
    let frame = FrameRaster::full(
        crate::geometry::Composition::new(
            [0, 0, 8, 4].into(),
            Default::default(),
            Default::default(),
        )
        .unwrap(),
    )
    .unwrap();
    let spec = SourceRequest::for_frame(frame).unwrap();
    let make = |renderer: Arc<crate::render_fixture::RenderFixture>, delta| {
        renderer
            .evaluation()
            .materializations()
            .set_retention_enabled(false);
        let request = Request::new(
            renderer.evaluation().clone(),
            kernels.clone(),
            vec![ImageRequest::new(endpoint(input.clone(), delta), spec.clone()).unwrap()],
        )
        .unwrap();
        let product = pollster::block_on(request.produce(&queue, || false))
            .unwrap()
            .unwrap()
            .remove(0);
        let capture = crate::photographic::graph::request::PhotographRequest::inspection(
            crate::gpu::parameters::ProcessingParameters::source(),
        )
        .unwrap();
        let target = crate::rendering::OutputTarget::Srgb;
        let endpoint = capture
            .display(
                product.definition.clone(),
                renderer.looks(),
                renderer.drt(target),
            )
            .unwrap();
        crate::gpu::view::RenderedView::from_value(&device, product, capture, target, endpoint)
            .unwrap()
    };
    let first = make(renderer.clone(), 3.);
    let independent =
        pollster::block_on(crate::render_fixture::RenderFixture::new(&device, &queue)).unwrap();
    let recomputed = make(independent, 3.);
    assert!(first.same_source_definition_as(&recomputed));
    assert!(!first.same_source_as(&recomputed));
    let reused = make(renderer.clone(), 3.);
    assert!(reused.same_source_as(&first));
    let edited = make(renderer, 5.);
    assert!(edited.same_source_definition_as(&first));
    assert!(
        !edited.same_source_as(&first),
        "completed frames do not pin source inputs at zero retention"
    );
    assert_eq!(reads.load(Ordering::Relaxed), 3);
}

#[test]
fn arbitrary_image_endpoints_share_the_recorded_and_produced_view_request() {
    let (device, queue) = pollster::block_on(async {
        let adapter = crate::gpu::instance()
            .request_adapter(&Default::default())
            .await
            .unwrap();
        crate::gpu::submission::request_device(&adapter, &Default::default())
            .await
            .unwrap()
    });
    let context = Context::new(
        &device,
        e::CachePolicy {
            generated_bytes: 1 << 20,
        },
    );
    let kernels = pollster::block_on(ViewKernels::new(&device)).unwrap();
    let scope = context.materializations();
    scope.set_retention_enabled(true);
    scope.set_test_pressure(crate::sys::MemoryPressure::Normal);
    let count = Arc::new(AtomicUsize::new(0));
    let input = e::source(Arc::new(Coordinates(count.clone())));
    let frame = FrameRaster::full(
        crate::geometry::Composition::new(
            [0, 0, 8, 4].into(),
            Default::default(),
            Default::default(),
        )
        .unwrap(),
    )
    .unwrap();
    let request = |definition| {
        Request::new(
            context.clone(),
            kernels.clone(),
            vec![ImageRequest::new(definition, SourceRequest::for_frame(frame).unwrap()).unwrap()],
        )
        .unwrap()
    };

    let cold = request(endpoint(input.clone(), 3.));
    assert!(cold.ready().is_none());
    let first = pollster::block_on(cold.produce(&queue, || false))
        .unwrap()
        .unwrap()
        .remove(0);
    assert_eq!(
        e::read_buffer(&device, &queue, &first.pixels)[0],
        [3., 0., -2., 8.]
    );
    assert_eq!(count.load(Ordering::Relaxed), 1);

    let selected = endpoint(input, 5.);
    let identity = selected
        .output(selected.domain().finite().unwrap())
        .unwrap()
        .identity()
        .unwrap()
        .clone();
    let hot = request(selected.clone());
    let next = hot.ready().unwrap().remove(0);
    assert!(next.definition.same_definition(&selected));
    assert!(
        scope
            .find_ready::<e::ImageLayout>(&identity, |_| true)
            .is_none(),
        "abandoned preparation cannot publish pixels"
    );
    let mut encoder = device.create_command_encoder(&Default::default());
    next.encode(&device, &mut encoder);
    queue.submit([encoder.finish()]);
    next.recording().take().unwrap().submitted();
    next.pixels.submitted(&scope);
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let pixels = e::read_buffer(&device, &queue, &next.pixels);
    assert_eq!(pixels[0], [5., 0., -2., 8.]);
    assert_eq!(pixels[31], [12., 3., -2., 8.]);
    assert_eq!(
        count.load(Ordering::Relaxed),
        1,
        "a different endpoint uses the live input without another source read"
    );
    assert!(context.retained_bytes() > 0);
    assert!(context.retained_bytes() <= 1 << 20);
    assert!(
        scope
            .find_ready::<e::ImageLayout>(&identity, |_| true)
            .is_some()
    );
}
