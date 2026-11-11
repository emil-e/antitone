use super::*;

#[test]
fn source_readiness_leaves_support_lazy_and_explicit_service_preserves_exact_samples() {
    pollster::block_on(async {
        let file = raw_fixture::bayer_dng();
        let document = document(file.path());
        let (device, queue) = gpu().await;
        let renderers = Arc::new(crate::renderer::RendererCache::default());
        let (_, reference) = prepare(&document, &document.cache, &device, &queue, &renderers);
        let expected = reference.propose_flat_field_area().unwrap();
        let before = codes(&reference);
        let scheduler = source_loading::SourceLoader::new();
        let services = source::Services {
            developments: scheduler.clone(),
            renderers,
            device,
            queue,
        };
        let id = document.project.order[0];
        let ready = services
            .prepare(
                source::Request {
                    source: document.source_input(id).unwrap(),
                    correction: None,
                    sources: Some(scheduler.provider(Some(document.cache.clone()))),
                },
                crate::cancellation::Cancellation::default(),
            )
            .await
            .unwrap();
        assert_eq!(ready.analysis_support, None);
        let input = source::Context {
            reference: None,
            source: ready.developed,
            correction: None,
        };
        let support = crate::background::run(move |_| input.scan()?.propose_flat_field_area())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            support, expected,
            "service extraction must preserve the existing exact estimator result"
        );
        assert_eq!(codes(&reference), before);
        scheduler.close_and_wait().unwrap();
    });
}
