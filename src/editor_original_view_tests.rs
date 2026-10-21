use super::*;

fn source_input() -> source::Context {
    source::Context {
        source: crate::editor::tests::developed_fixture(),
        correction: None,
    }
}

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
fn original_demand_captures_purpose_and_latest_input_and_rejects_stale_completion() {
    let mut owner = OriginalView::new(Arc::default());
    let mut scope = super::super::grade_scope::GradeScope::default();
    let context = egui::Context::default();
    let mut retired = None;
    for i in 0..40 {
        let (signature, input) = request(i as f64 / 10.);
        owner.observe(OriginalPurpose::Tool, signature);
        owner.last_edit = std::time::Instant::now() - Duration::from_secs(1);
        owner.schedule(
            &input,
            OutputTarget::Srgb,
            &mut scope,
            &context,
            source_input(),
        );
        let captured = scope.request.as_ref().unwrap();
        assert_eq!(captured.input.signature(), input.signature());
        assert_eq!(owner.refining.unwrap().0, captured.revision);
        retired.get_or_insert(captured.revision);
    }
    let running = owner.refining.unwrap();
    assert!(owner.receive(retired.unwrap(), Ok(None)).is_none());
    assert_eq!(owner.refining, Some(running));
    owner.leave();
    assert!(owner.signature.is_none() && owner.refining.is_none());
}

#[test]
fn ordinary_views_without_working_device_never_become_original_tool_requests() {
    let mut editor = super::super::tests::fixture();
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
        assert!(editor.document.visit.grade.request.is_none());
    }
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
fn original_failure_latches_once_and_rejects_unrequested_completions() {
    let mut owner = OriginalView::new(Arc::default());
    let mut scope = super::super::grade_scope::GradeScope::default();
    let (signature, input) = request(0.);
    owner.observe(OriginalPurpose::Tool, signature);
    owner.last_edit = std::time::Instant::now() - Duration::from_secs(1);
    let context = egui::Context::default();
    owner.schedule(
        &input,
        OutputTarget::Srgb,
        &mut scope,
        &context,
        source_input(),
    );
    let revision = scope.request.take().unwrap().revision;
    assert_eq!(
        owner
            .receive(revision, Err("source unavailable".into()))
            .as_deref(),
        Some("Original view failed: source unavailable")
    );
    owner.schedule(
        &input,
        OutputTarget::Srgb,
        &mut scope,
        &context,
        source_input(),
    );
    assert!(scope.request.is_none());
    assert!(owner.receive(revision, Ok(None)).is_none());
    assert_eq!(owner.refinement_error, Some(revision));
}
