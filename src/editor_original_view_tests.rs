use super::*;

fn request(ev: f64) -> (Signature, ProcessingInput) {
    let input = ProcessingInput::from(ProcessingParameters::source_inspection(ev).unwrap());
    let area = Rect {
        x: 0,
        y: 0,
        width: 4,
        height: 3,
    };
    let view = ViewRequest {
        area,
        output: [4, 3],
        frame: None,
    };
    (
        (
            input.signature(),
            OutputTarget::Srgb,
            Some(view),
            Composition::new(area, Default::default(), Default::default()).unwrap(),
        ),
        input,
    )
}

#[test]
fn original_jobs_are_purpose_bound_and_cancelled_work_keeps_admission_until_completion() {
    let mut owner = OriginalView::new(Arc::default());
    let (commands, jobs) = mpsc::channel();
    let context = egui::Context::default();
    let (signature, input) = request(0.);
    owner.observe(OriginalPurpose::Tool, signature);
    owner.last_edit = std::time::Instant::now() - Duration::from_secs(1);
    assert!(
        owner
            .schedule(1, &input, OutputTarget::Srgb, &commands, &context)
            .is_none()
    );
    let running = owner.revision();
    assert!(
        matches!(jobs.try_recv().unwrap(),Command::Refine(_,revision,..) if revision == running)
    );
    for i in 1..=40 {
        let (signature, input) = request(i as f64 / 10.);
        owner.observe(OriginalPurpose::Tool, signature);
        owner.last_edit = std::time::Instant::now() - Duration::from_secs(1);
        assert!(
            owner
                .schedule(1, &input, OutputTarget::Srgb, &commands, &context)
                .is_none()
        );
        assert_eq!(owner.refining.unwrap().0, running);
    }
    assert!(jobs.try_recv().is_err());
    assert!(owner.receive(owner.revision(), Ok(None)).is_none());
    assert!(
        owner.refining.is_some(),
        "unmatched current-generation completion cannot steal the old admission token"
    );
    assert!(owner.receive(running, Ok(None)).is_none());
    assert!(owner.refining.is_none());
    let (_, input) = request(4.);
    assert!(
        owner
            .schedule(1, &input, OutputTarget::Srgb, &commands, &context)
            .is_none()
    );
    assert!(
        matches!(jobs.try_recv().unwrap(),Command::Refine(_,revision,..) if revision > running)
    );
    owner.leave();
    assert!(owner.signature.is_none());
    assert!(
        owner.refining.is_some(),
        "leaving a tool cancels, but completion releases admission"
    );
}

#[test]
fn ordinary_views_without_working_device_never_become_original_tool_requests() {
    let (mut editor, _, commands) = super::super::tests::fixture();
    super::super::tests::add_ready_fixture(&mut editor);
    let context = egui::Context::default();
    assert!(!editor.working.enabled());
    for source in [false, true] {
        editor.source = source;
        for _ in 0..3 {
            editor.original.last_edit = std::time::Instant::now() - Duration::from_secs(1);
            super::super::tests::app_frame(&mut editor, &context, vec![]);
        }
        assert!(editor.original.signature.is_none());
        assert!(editor.original.refining.is_none());
    }
    assert!(
        !commands
            .try_iter()
            .any(|c| matches!(c, Command::Refine(..)))
    );
}

#[test]
fn leaving_unbuilt_reference_releases_idle_authority_without_revision_churn() {
    let mut owner = OriginalView::new(Arc::default());
    owner.observe(OriginalPurpose::Reference, request(0.).0);
    owner.leave();
    assert!(owner.signature.is_none());
    let revision = owner.revision();
    for _ in 0..40 {
        owner.leave();
    }
    assert_eq!(owner.revision(), revision);
    owner.last_edit = std::time::Instant::now() - Duration::from_secs(1);
    assert!(owner.idle());
}

#[test]
fn original_worker_failure_latches_once_and_rejects_unrequested_completions() {
    let mut owner = OriginalView::new(Arc::default());
    let (commands, jobs) = mpsc::channel();
    drop(jobs);
    let (signature, input) = request(0.);
    owner.observe(OriginalPurpose::Tool, signature);
    owner.last_edit = std::time::Instant::now() - Duration::from_secs(1);
    let context = egui::Context::default();
    assert_eq!(
        owner.schedule(1, &input, OutputTarget::Srgb, &commands, &context),
        Some("RAW worker stopped")
    );
    assert!(
        owner
            .schedule(1, &input, OutputTarget::Srgb, &commands, &context)
            .is_none()
    );
    assert!(owner.receive(owner.revision(), Ok(None)).is_none());
    assert_eq!(owner.refinement_error, Some(owner.revision()));
}
