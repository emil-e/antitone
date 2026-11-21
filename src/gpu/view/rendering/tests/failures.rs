use super::*;

#[test]
fn worker_production_preserves_memory_failure_without_publishing_a_view() {
    struct FailedAllocation;
    impl e::Source for FailedAllocation {
        fn domain(&self) -> e::Domain {
            Coordinates(Arc::new(AtomicUsize::new(0))).domain()
        }
        fn identity(&self) -> Option<e::RetainedKey> {
            Some(e::RetainedKey::new("failing view allocation"))
        }
        fn read(&self, _: e::CellRegion) -> Result<Vec<[f32; 4]>, Error> {
            Err(Error::GpuOutOfMemory(
                "injected source upload failure".into(),
            ))
        }
    }
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let kernels = pollster::block_on(ViewKernels::new(&device)).unwrap();
    let frame = FrameRaster::full(
        crate::geometry::Composition::new(
            [0, 0, 8, 4].into(),
            Default::default(),
            Default::default(),
        )
        .unwrap(),
    )
    .unwrap();
    let request = Request::new(
        Preparation::new(
            renderer.clone(),
            kernels,
            vec![
                ImageRequest::new(
                    endpoint(e::source(Arc::new(FailedAllocation)), 3.),
                    SourceRequest::for_frame(frame).unwrap(),
                )
                .unwrap(),
            ],
        )
        .unwrap(),
    )
    .unwrap();
    assert!(request.ready().is_none());
    let result = pollster::block_on(request.produce(&queue, || false));
    assert!(matches!(result, Err(Error::GpuOutOfMemory(_))));
    assert!(
        request.ready().is_none(),
        "a failed production cannot publish an image"
    );
}
