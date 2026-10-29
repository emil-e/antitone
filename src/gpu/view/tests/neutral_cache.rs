use super::*;
use crate::{
    acquisition::Sampling,
    pixel_pipeline::{PixelPipeline, PixelStage},
};
#[test]
fn completed_neutral_images_reuse_larger_support_with_content_capabilities_not_publication() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let scan = scan();
        let frame = frame(&scan);
        let large = ViewSourceSpec::new(7, frame, calibration(), scan.camera_to_ap1, None)
            .unwrap()
            .with_pipeline(PixelPipeline {
                stages: vec![PixelStage::new(crate::image_mtf::ImageMtfParameters {
                    sigma: 1.,
                })],
            })
            .unwrap();
        let source = ViewSource::acquire(
            &device,
            &queue,
            &Default::default(),
            renderer.clone(),
            kernels.clone(),
            &scan,
            None,
            large,
            || false,
        )
        .unwrap()
        .unwrap();
        let cache = renderer.evaluation().image_cache(2);
        assert!(source.retain_neutral(&cache));
        let mut wanted = source
            .spec
            .clone()
            .with_pipeline(Default::default())
            .unwrap();
        wanted.authority = 99;
        let found =
            ViewSource::cached_neutral(&cache, renderer.clone(), kernels.clone(), &wanted).unwrap();
        assert_eq!(found.spec.authority, 99);
        assert!(found.same_pixels_as(&source));
        assert_eq!(
            found.spec.input_frame().unwrap(),
            source.spec.input_frame().unwrap()
        );
        assert_ne!(
            found.spec.input_frame().unwrap(),
            wanted.input_frame().unwrap()
        );
        let fresh = ViewSource::acquire(
            &device,
            &queue,
            &Default::default(),
            renderer.clone(),
            kernels.clone(),
            &scan,
            None,
            wanted.clone(),
            || false,
        )
        .unwrap()
        .unwrap();
        let input = processing_input(
            &device,
            calibration(),
            Default::default(),
            Default::default(),
            &scan,
        );
        let from_cached =
            RenderedView::new(&device, found.clone(), &input, OutputTarget::Srgb, None).unwrap();
        let from_fresh =
            RenderedView::new(&device, fresh.clone(), &input, OutputTarget::Srgb, None).unwrap();
        let context = egui::Context::default();
        let gate = SubmissionGate::default();
        from_cached.ensure_submitted(&device, &queue, &gate, &context);
        from_fresh.ensure_submitted(&device, &queue, &gate, &context);
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(10)),
            })
            .unwrap();
        assert_eq!(
            image(&device, &queue, &from_cached.image),
            image(&device, &queue, &from_fresh.image)
        );
        let mut changed = wanted.clone();
        changed.request.matrix *= 1.01;
        assert!(
            ViewSource::cached_neutral(&cache, renderer.clone(), kernels.clone(), &changed)
                .is_none()
        );
        let moved = wanted
            .clone()
            .with_frame(FrameRaster {
                output: [frame.output[0] / 2, frame.output[1] / 2],
                ..frame
            })
            .unwrap();
        assert!(
            ViewSource::cached_neutral(&cache, renderer.clone(), kernels.clone(), &moved).is_none()
        );
        let proxy = wanted
            .with_sampling(Sampling::CameraProxy {
                region: scan.area,
                size: [8, 8],
            })
            .unwrap();
        assert!(
            ViewSource::cached_neutral(&cache, renderer.clone(), kernels.clone(), &proxy).is_none()
        );
        let foreign = crate::gpu::evaluation::Context::new(
            &device,
            crate::gpu::evaluation::CachePolicy { generated_bytes: 0 },
        )
        .image_cache(2);
        assert!(!source.retain_neutral(&foreign));
        let off_renderer = crate::renderer::GpuRenderer::new_with_cache_policy(
            &device,
            &queue,
            Default::default(),
            crate::gpu::evaluation::CachePolicy { generated_bytes: 0 },
        )
        .await
        .unwrap();
        let off = ViewSource::acquire(
            &device,
            &queue,
            &Default::default(),
            off_renderer.clone(),
            kernels.clone(),
            &scan,
            None,
            source.spec.clone(),
            || false,
        )
        .unwrap()
        .unwrap();
        let off_cache = off_renderer.evaluation().image_cache(2);
        assert!(!off.retain_neutral(&off_cache));
        assert!(off_cache.is_empty());
        assert_eq!(off_renderer.evaluation().retained_bytes(), 0);
        assert_eq!(
            read(&device, &queue, &off.pixels),
            read(&device, &queue, &source.pixels)
        );
        drop((off, off_renderer));
        let active = found.pixels.clone();
        let bytes = source.pixels.size();
        let evaluation = renderer.evaluation().clone();
        assert_eq!(evaluation.retained_bytes(), bytes);
        let owner = Arc::downgrade(&renderer);
        drop((source, found, fresh, from_cached, from_fresh, renderer));
        assert!(
            owner.upgrade().is_none(),
            "retained pixels must not own their renderer"
        );
        cache.clear();
        assert_eq!(evaluation.retained_bytes(), 0);
        assert!(
            !read(&device, &queue, &active).is_empty(),
            "retirement must preserve active pixel consumers"
        );
    });
}
