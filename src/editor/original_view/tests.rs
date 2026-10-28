use super::*;
use crate::editor::tests::UiFixture;

fn source_input() -> source::Context {
    source::Context {
        reference: None,
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
fn original_request_is_readonly_and_policy_rejects_departed_completion() {
    let mut owner = OriginalView::new(Arc::default());
    let mut retired = None;
    for i in 0..40 {
        let (signature, input) = request(i as f64 / 10.);
        owner.observe(OriginalPurpose::Tool, signature);
        owner.last_edit = std::time::Instant::now() - Duration::from_secs(1);
        let captured = owner
            .request(&input, OutputTarget::Srgb, source_input())
            .unwrap();
        assert_eq!(captured.input.signature(), input.signature());
        assert!(
            owner.refining.is_none(),
            "building input does not stage work"
        );
        // Policy-only producer event; real task registration/retirement has workflow coverage.
        owner.refining = Some((captured.revision, captured.view));
        retired.get_or_insert(captured.revision);
    }
    let running = owner.refining.unwrap();
    assert!(owner.receive(retired.unwrap(), Ok(None)).is_none());
    assert_eq!(owner.refining, Some(running));
    owner.leave();
    assert!(owner.signature.is_none() && owner.refining.is_none());
}

#[test]
fn ordinary_views_never_become_original_tool_requests() {
    let mut editor = super::super::tests::fixture();
    super::super::tests::add_ready_fixture(&mut editor);
    let context = egui::Context::default();
    assert!(editor.document.visit.working_view.enabled());
    for source in [false, true] {
        editor.document.source = source;
        for _ in 0..3 {
            editor.document.visit.grade.original.last_edit =
                std::time::Instant::now() - Duration::from_secs(1);
            super::super::tests::app_frame(&mut editor, &context, vec![]);
        }
        assert!(
            editor.document.visit.grade.original.signature.is_none()
                && editor.document.visit.grade.original.refining.is_none()
        );
        assert!(editor.document.visit.grade.task.is_none());
    }
}

#[test]
fn an_uninitialized_viewer_does_not_paint_through_an_alternate_proxy_renderer() {
    let mut editor = super::super::tests::fixture();
    super::super::tests::add_ready_fixture(&mut editor);
    editor.document.visit.working_view = WorkingView::default();
    editor.document.source = true;
    editor.scopes.visible = false;
    let context = egui::Context::default();
    presentation::configure_fonts(&context);
    for _ in 0..3 {
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100., 800.),
                )),
                ..Default::default()
            },
            |ui| editor.ui(ui),
        );
        output.textures_delta.clear();
        assert!(
            !output
                .shapes
                .iter()
                .any(|shape| matches!(shape.shape, egui::Shape::Callback(_))),
            "an unavailable shared renderer must not silently select a second image-processing path"
        );
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
    let (signature, input) = request(0.);
    owner.observe(OriginalPurpose::Tool, signature);
    owner.last_edit = std::time::Instant::now() - Duration::from_secs(1);
    let captured = owner
        .request(&input, OutputTarget::Srgb, source_input())
        .unwrap();
    let revision = captured.revision;
    owner.refining = Some((revision, captured.view));
    assert_eq!(
        owner
            .receive(revision, Err("source unavailable".into()))
            .as_deref(),
        Some("Original view failed: source unavailable")
    );
    assert!(
        owner
            .request(&input, OutputTarget::Srgb, source_input())
            .is_none()
    );
    assert!(owner.receive(revision, Ok(None)).is_none());
    assert_eq!(owner.refinement_error, Some(revision));
}
