use super::*;
use std::sync::atomic::AtomicU64;

#[test]
fn worker_computes_exact_support_only_after_a_current_source_request() {
    pollster::block_on(async {
        let file = raw_fixture::bayer_dng();
        let document = document(file.path());
        let (device, queue) = gpu().await;
        let renderers = Arc::new(crate::renderer::RendererCache::default());
        let (_, reference) = prepare(&document, &device, &queue, &renderers);
        let expected = reference.propose_flat_field_area().unwrap();
        let before = codes(&reference);
        let epoch = Arc::new(AtomicU64::new(1));
        let (commands, receiver) = mpsc::channel();
        let (send, messages) = mpsc::channel();
        let scheduler = development::Scheduler::new();
        let worker_scheduler = scheduler.clone();
        let worker_epoch = epoch.clone();
        let worker = std::thread::spawn(move || {
            backend(
                renderers,
                device,
                (queue, crate::platform::SubmissionGate::default()),
                receiver,
                send,
                Arc::default(),
                Arc::default(),
                worker_scheduler,
                worker_epoch,
            )
        });
        let id = document.project.order[0];
        let path = document.source_path(id).unwrap();
        let source = document.project.source(id).unwrap();
        commands
            .send(Command::Load(
                1,
                path.clone(),
                wb(&document),
                Some(Box::new(source.clone())),
                None,
                document.project.camera_basis.clone(),
                LoadCache::new(&path, Some(source), wb(&document), &document),
            ))
            .unwrap();
        match messages.recv_timeout(Duration::from_secs(5)).unwrap() {
            Message::Ready(1, Ok(ready)) => assert_eq!(ready.analysis_support, None),
            _ => panic!("source did not become ready independently of support"),
        }
        commands.send(Command::AnalysisSupport(2, 1)).unwrap();
        match messages.recv_timeout(Duration::from_secs(5)).unwrap() {
            Message::AnalysisSupport(2, 1, Ok(support)) => assert_eq!(support, expected),
            _ => panic!("support differed from original exact algorithm"),
        }
        epoch.store(3, std::sync::atomic::Ordering::Relaxed);
        commands.send(Command::AnalysisSupport(4, 1)).unwrap();
        assert!(matches!(
            messages.recv_timeout(Duration::from_secs(5)).unwrap(),
            Message::AnalysisSupport(4, 1, Err(_))
        ));
        assert_eq!(codes(&reference), before);
        drop(commands);
        worker.join().unwrap();
        scheduler.close();
    });
}
