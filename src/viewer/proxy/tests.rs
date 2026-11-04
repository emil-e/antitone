use super::*;
use crate::gpu::parameters::ProcessingParameters;
use crate::gpu::source::ProcessingSource;
use crate::gpu::source::SourceGeometry;
use crate::gpu::test_inputs::AuthoredPhotograph;

fn shared_source(image: &GpuImage) -> &Arc<ProcessingSource> {
    &image.processing.as_ref().unwrap().source
}

#[test]
fn selected_proxy_has_one_immutable_source_and_lazy_active_target_output() {
    pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let renderer = GpuRenderer::new(&device, &queue).await.unwrap();
        let size = [32, 16];
        let count = size[0] * size[1];
        let proxy = GpuProxy::new(
            renderer,
            &device,
            size,
            &vec![[1000, 1200, 5000, 0]; count as usize],
            DMat3::IDENTITY,
            SourceGeometry::new([12, 7, 864, 576], size).unwrap(),
        )
        .unwrap();
        let source_bytes = u64::from(count) * 8;
        assert_eq!(
            proxy.byte_len(),
            source_bytes,
            "no unused target/working output is allocated"
        );
        let srgb = proxy.image(OutputTarget::Srgb).unwrap();
        assert!(Arc::ptr_eq(
            &srgb,
            &proxy.image(OutputTarget::Srgb).unwrap()
        ));
        let srgb_owner = Arc::downgrade(&srgb);
        let p3 = proxy.image(OutputTarget::DisplayP3).unwrap();
        let a = srgb.processing.as_ref().unwrap();
        let b = p3.processing.as_ref().unwrap();
        assert!(Arc::ptr_eq(shared_source(&srgb), shared_source(&p3)));
        assert_ne!(a.working, b.working);
        assert_eq!(
            proxy.byte_len(),
            source_bytes + u64::from(count) * 32,
            "one active working/target pair; rendering constants are prepared with an actual calculation, and held consumers are separate"
        );
        assert!(
            !shared_source(&srgb)
                .input
                .usage()
                .contains(wgpu::BufferUsages::COPY_DST)
        );
        let source = AuthoredPhotograph::from(ProcessingParameters::source());
        let changed =
            AuthoredPhotograph::from(ProcessingParameters::source_inspection(1.).unwrap());
        let mut encoder = device.create_command_encoder(&Default::default());
        assert!(
            srgb.prepare_processing(&device, &mut encoder, &source)
                .unwrap()
        );
        assert!(
            p3.prepare_processing(&device, &mut encoder, &changed)
                .unwrap()
        );
        assert!(
            !srgb
                .prepare_processing(&device, &mut encoder, &source)
                .unwrap(),
            "target changes must not alias another image's preparation"
        );
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        drop(srgb);
        assert!(
            srgb_owner.upgrade().is_none(),
            "inactive target has no hidden owner"
        );
        let p3_owner = Arc::downgrade(&p3);
        proxy.release_output();
        assert_eq!(proxy.byte_len(), source_bytes);
        assert!(
            p3_owner.upgrade().is_some(),
            "existing encoded/presentation consumers remain valid"
        );
        let fresh = proxy.image(OutputTarget::DisplayP3).unwrap();
        assert!(!Arc::ptr_eq(&fresh, &p3));
        assert!(Arc::ptr_eq(shared_source(&fresh), shared_source(&p3)));
        drop(p3);
        assert!(p3_owner.upgrade().is_none());
        drop(proxy);
        let last = Arc::try_unwrap(fresh).ok().unwrap();
        assert!(
            !shared_source(&last)
                .input
                .usage()
                .contains(wgpu::BufferUsages::COPY_DST)
        );
    });
}

#[cfg(feature = "editor")]
#[test]
fn proxy_preview_validates_the_complete_calculation_before_native_submission() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let kernels = pollster::block_on(crate::gpu::view::ViewKernels::new(&device)).unwrap();
    let proxy = GpuProxy::new(
        renderer,
        &device,
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
    let gate = crate::gpu::submission::SubmissionGate::default();
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
                &queue,
                &gate,
                kernels.clone(),
                &crate::gpu::photograph::request::PhotographRequest::Fixture(invalid),
                None,
                spec.clone()
            )
            .is_err()
    );
    assert_eq!(
        count.load(Ordering::Relaxed),
        0,
        "a late downstream error must not leave native work submitted"
    );
    let valid = AuthoredPhotograph::from(ProcessingParameters::source());
    let (source, processed) = proxy
        .preview(
            &queue,
            &gate,
            kernels,
            &crate::gpu::photograph::request::PhotographRequest::Fixture(valid.clone()),
            None,
            spec,
        )
        .unwrap();
    assert_eq!(count.load(Ordering::Relaxed), 1);
    assert!(processed.stages.is_none());
    assert!(*processed.submitted.lock().unwrap());
    assert!(
        processed
            .definition
            .region_for(
                &source.definition,
                processed.definition.domain().finite().unwrap()
            )
            .unwrap()
            .is_some()
    );
    let rendered = crate::gpu::view::RenderedView::from_processed(
        &device,
        source,
        crate::gpu::photograph::request::PhotographRequest::Fixture(valid),
        OutputTarget::Srgb,
        processed,
        None,
    )
    .unwrap();
    rendered.ensure_submitted(&device, &queue, &gate, &egui::Context::default());
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
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let renderer = GpuRenderer::new(&device, &queue).await.unwrap();
        let kernels = crate::gpu::view::ViewKernels::new(&device).await.unwrap();
        let size = [32, 16];
        let proxy = GpuProxy::new_continuous(
            renderer,
            &device,
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
        .unwrap()
        .with_repairs(Default::default());
        let source = proxy
            .view_source(
                &queue,
                &crate::gpu::submission::SubmissionGate::default(),
                kernels,
                &spec.processing_fixture(),
                spec,
            )
            .unwrap();
        assert_eq!(source.spec.frame().output, [16, 8]);
    });
}
