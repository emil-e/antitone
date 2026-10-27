use super::*;

#[test]
fn source_readiness_leaves_support_lazy_and_explicit_service_preserves_exact_samples() {
    pollster::block_on(async {
        let file = raw_fixture::bayer_dng();
        let document = document(file.path());
        let (device, queue) = gpu().await;
        let renderers = Arc::new(crate::renderer::RendererCache::default());
        let (_, reference) = prepare(&document, &device, &queue, &renderers);
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
        let path = document.source_path(id).unwrap();
        let source = document.project.source(id).unwrap();
        let ready = services
            .prepare(
                source::Request {
                    input: document.project.input,
                    path: path.clone(),
                    wb: wb(&document),
                    expected: Some(source.clone()),
                    correction: None,
                    cache: Some(LoadCache::new(&path, Some(source), &document)),
                },
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
            )
            .await
            .unwrap();
        assert_eq!(ready.analysis_support, None);
        let support = support_workflow::prepare(source::Context {
            source: ready.developed,
            correction: None,
        })
        .await
        .unwrap();
        assert_eq!(
            support, expected,
            "service extraction must preserve the existing exact estimator result"
        );
        assert_eq!(codes(&reference), before);
        scheduler.close_and_wait().unwrap();
    });
}
