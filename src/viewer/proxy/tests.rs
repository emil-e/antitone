use super::*;

fn shared_source(image: &GpuImage) -> &Arc<ProcessingSource> {
    let ImageSource::Shared(source) = &image.processing.as_ref().unwrap().source else {
        panic!("shared proxy")
    };
    source
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
            None,
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
            source_bytes + u64::from(count) * 32 + std::mem::size_of::<[u32; 4]>() as u64,
            "one active working/target pair plus its captured dimensions; held consumers are separate"
        );
        assert!(
            srgb.write_source(&queue, &vec![[0; 4]; count as usize])
                .is_err(),
            "shared camera pixels cannot be rewritten"
        );
        let source = ProcessingInput::from(ProcessingParameters::source());
        let changed = ProcessingInput::from(ProcessingParameters::source_inspection(1.).unwrap());
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
        let mut last = Arc::try_unwrap(fresh).ok().unwrap();
        assert!(
            last.write_source(&queue, &vec![[0; 4]; count as usize])
                .is_err(),
            "shared authority stays immutable after all other owners retire"
        );
        let geometry = SourceGeometry::new([0, 0, 32, 16], size).unwrap();
        assert!(last.set_geometry(geometry).is_err());
        assert!(last.set_flat_field(None, geometry).is_err());
    });
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
            None,
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
        let spec = crate::gpu::view::ViewSourceSpec::new(
            1,
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
                &crate::gpu::SubmissionGate::default(),
                kernels,
                spec,
            )
            .unwrap();
        assert_eq!(source.spec.frame().output, [16, 8]);
    });
}
