use super::*;
use crate::gpu::parameters::ProcessingParameters;
use crate::gpu::source::SourceGeometry;
use crate::gpu::test_inputs::AuthoredPhotograph;
#[cfg(feature = "editor")]
use crate::render_fixture::RenderFixture;

#[cfg(feature = "editor")]
#[test]
fn proxy_products_share_immutable_input_without_retaining_each_other_or_the_proxy() {
    use crate::gpu::photograph::{PhotographGraph, request::PhotographRequest};
    use crate::gpu::view::{RenderedView, ViewKernels, request::SourceRequest};
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let kernels = pollster::block_on(ViewKernels::new(&device)).unwrap();
    let size = [32, 16];
    let proxy = GpuProxy::new(
        renderer.evaluation().clone(),
        renderer.looks().clone(),
        renderer.drt.clone(),
        size,
        &vec![[1000, 1200, 5000, 0]; (size[0] * size[1]) as usize],
        DMat3::IDENTITY,
        SourceGeometry::new([12, 7, 864, 576], size).unwrap(),
    )
    .unwrap();
    let source_bytes = u64::from(size[0] * size[1]) * 8;
    assert_eq!(proxy.byte_len(), source_bytes);
    assert!(
        !proxy
            .0
            .source
            .input
            .usage()
            .contains(wgpu::BufferUsages::COPY_DST)
    );
    let source = Arc::downgrade(&proxy.0.source);
    let composition = crate::geometry::Composition::new(
        [12, 7, 864, 576].into(),
        Default::default(),
        Default::default(),
    )
    .unwrap();
    let mut frame = crate::frame_raster::FrameRaster::full(composition).unwrap();
    frame.output = size;
    let spec = SourceRequest::inspection(frame).unwrap();
    let make = |target, parameters| {
        let capture = PhotographRequest::Fixture(AuthoredPhotograph::from(parameters));
        let request = proxy
            .preview(kernels.clone(), &capture, None, spec.clone(), target)
            .unwrap();
        let image = request.ready().expect("small resident proxy").remove(0);
        RenderedView::from_image(&device, image, capture, target).unwrap()
    };
    let srgb = make(OutputTarget::Srgb, ProcessingParameters::source());
    let p3 = make(
        OutputTarget::DisplayP3,
        ProcessingParameters::source_inspection(1.).unwrap(),
    );
    assert!(PhotographGraph::shares_resident_camera(
        &srgb.product().unwrap().definition,
        &p3.product().unwrap().definition
    ));
    assert_ne!(
        &*srgb.product().unwrap().pixels,
        &*p3.product().unwrap().pixels
    );
    assert_eq!(
        proxy.byte_len(),
        source_bytes,
        "products are owned by their consumers, not a hidden active-target slot"
    );
    let srgb_owner = Arc::downgrade(&srgb);
    drop(srgb);
    assert!(srgb_owner.upgrade().is_none());
    drop(proxy);
    assert!(
        source.upgrade().is_none(),
        "the endpoint captures pixels, not the proxy owner"
    );
    p3.ensure_submitted(&device, &queue, &egui::Context::default());
    let pixels = crate::viewer::readback::image_pixels(&device, &queue, &p3.image);
    assert_eq!(pixels.len(), (size[0] * size[1]) as usize);
    assert!(
        pixels
            .iter()
            .all(|pixel| pixel[3] == 1. && pixel[..3].iter().all(|v| v.is_finite() && *v > 0.))
    );
}

#[cfg(feature = "editor")]
#[test]
fn proxy_preview_validates_the_complete_calculation_before_native_submission() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let kernels = pollster::block_on(crate::gpu::view::ViewKernels::new(&device)).unwrap();
    let proxy = GpuProxy::new(
        renderer.evaluation().clone(),
        renderer.looks().clone(),
        renderer.drt.clone(),
        [2, 2],
        &[[32000, 22000, 16000, 0]; 4],
        DMat3::IDENTITY,
        SourceGeometry::new([0, 0, 2, 2], [2, 2]).unwrap(),
    )
    .unwrap();
    let composition = crate::geometry::Composition::new(
        [0, 0, 2, 2].into(),
        Default::default(),
        Default::default(),
    )
    .unwrap();
    let frame = crate::frame_raster::FrameRaster::full(composition).unwrap();
    let spec = crate::gpu::view::request::SourceRequest::inspection(frame).unwrap();
    let gate = queue.clone();
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    gate.observe_submissions(Arc::new(move || {
        observed.fetch_add(1, Ordering::Relaxed);
    }));
    let invalid = AuthoredPhotograph::from(
        ProcessingParameters::source_preview_with_transform(None, 0., DMat3::ZERO).unwrap(),
    );
    assert!(
        proxy
            .preview(
                kernels.clone(),
                &crate::gpu::photograph::request::PhotographRequest::Fixture(invalid),
                None,
                spec.clone(),
                OutputTarget::Srgb,
            )
            .is_err()
    );
    assert_eq!(
        count.load(Ordering::Relaxed),
        0,
        "a late downstream error must not leave native work submitted"
    );
    let valid = AuthoredPhotograph::from(ProcessingParameters::source());
    let request = proxy
        .preview(
            kernels,
            &crate::gpu::photograph::request::PhotographRequest::Fixture(valid.clone()),
            None,
            spec,
            OutputTarget::Srgb,
        )
        .unwrap();
    assert_eq!(count.load(Ordering::Relaxed), 0);
    let source = request.ready().expect("small resident proxy").remove(0);
    assert!(source.commands.is_some());
    assert!(!*source.submitted.lock().unwrap());
    assert!(crate::gpu::evaluation::has_inspection(&source.definition));
    let rendered = crate::gpu::view::RenderedView::from_image(
        &device,
        source,
        crate::gpu::photograph::request::PhotographRequest::Fixture(valid),
        OutputTarget::Srgb,
    )
    .unwrap();
    rendered.ensure_submitted(&device, &queue, &egui::Context::default());
    assert_eq!(count.load(Ordering::Relaxed), 1);
    let pixels = crate::viewer::readback::image_pixels(&device, &queue, &rendered.image);
    assert_eq!(pixels.len(), 4);
    assert!(pixels.iter().all(|pixel| {
        pixel[3] == 1.
            && pixel[..3]
                .iter()
                .all(|value| value.is_finite() && *value > 0.)
    }));
}

#[cfg(feature = "editor")]
#[test]
fn proxy_view_source_accepts_a_cropped_frame_output() {
    pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = crate::gpu::submission::request_device(&adapter, &Default::default())
            .await
            .unwrap();
        let renderer = RenderFixture::new(&device, &queue).await.unwrap();
        let kernels = crate::gpu::view::ViewKernels::new(&device).await.unwrap();
        let size = [32, 16];
        let proxy = GpuProxy::new_continuous(
            renderer.evaluation().clone(),
            renderer.looks().clone(),
            renderer.drt.clone(),
            size,
            &vec![[4000., 5000., 6000., 0.]; (size[0] * size[1]) as usize],
            DMat3::IDENTITY,
            SourceGeometry::new([0, 0, size[0], size[1]], size).unwrap(),
        )
        .unwrap();
        let area = crate::geometry::SourceRegion {
            x: 0,
            y: 0,
            width: size[0],
            height: size[1],
        };
        let composition = crate::geometry::Composition::with_crop(
            area,
            Default::default(),
            Default::default(),
            Some(crate::crop::Crop::from_region(crate::raw::Rect {
                x: 8,
                y: 4,
                width: 16,
                height: 8,
            })),
        )
        .unwrap();
        let frame = crate::frame_raster::FrameRaster::full(composition).unwrap();
        let spec = crate::gpu::view::request::SourceRequest::new(
            frame,
            crate::processing::InputSettings::Digital,
            DMat3::IDENTITY,
            None,
        )
        .unwrap();
        let source = proxy
            .view_source(&queue, kernels, &spec.processing_fixture(), spec)
            .unwrap();
        assert_eq!(source.spec.frame().output, [16, 8]);
    });
}
