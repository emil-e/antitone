use super::*;
use crate::editor::tests::UiFixture;
use crate::gpu::parameters::ProcessingParameters;
use crate::gpu::test_inputs::AuthoredPhotograph;

fn source_input() -> source::Context {
    source::Context {
        reference: None,
        source: crate::editor::tests::developed_fixture(),
        correction: None,
    }
}

fn request(ev: f64) -> (Demand, AuthoredPhotograph) {
    let input = AuthoredPhotograph::from(ProcessingParameters::source_inspection(ev).unwrap());
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
        Demand::fixture(
            input.clone(),
            OutputTarget::Srgb,
            Some(view),
            Composition::new(area, Default::default(), Default::default()).unwrap(),
            source_input(),
        ),
        input,
    )
}

#[test]
fn original_request_is_readonly_and_its_owner_rejects_departed_completion() {
    let mut owner = OriginalView::default();
    let mut retired = None;
    for i in 0..40 {
        let (signature, input) = request(i as f64 / 10.);
        owner.observe(OriginalPurpose::Tool, signature);
        owner.last_edit = std::time::Instant::now() - Duration::from_secs(1);
        let captured = owner.request().unwrap();
        assert!(
            captured.demand.capture
                == crate::gpu::photograph::request::PhotographRequest::Fixture(input)
        );
        assert!(!owner.refining(), "capturing input does not stage work");
        let completion = captured.begin().unwrap();
        retired.get_or_insert(completion);
    }
    assert!(retired.unwrap().receive(Err("departed".into())).is_none());
    assert!(owner.refining() && !owner.failed());
    owner.leave();
    assert!(owner.wanted.is_none() && !owner.refining());
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
            editor.document.visit.canvas.grade.original.last_edit =
                std::time::Instant::now() - Duration::from_secs(1);
            super::super::tests::app_frame(&mut editor, &context, vec![]);
        }
        assert!(
            editor.document.visit.canvas.grade.original.wanted.is_none()
                && !editor.document.visit.canvas.grade.original.refining()
        );
        assert!(editor.document.visit.canvas.grade.original.task.is_none());
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
fn leaving_unbuilt_reference_releases_its_continuation_without_idle_owner_churn() {
    let mut owner = OriginalView::default();
    owner.observe(OriginalPurpose::Reference, request(0.).0);
    owner.leave();
    assert!(owner.wanted.is_none());
    let continuation = owner.continuation();
    for _ in 0..40 {
        owner.leave();
    }
    assert_eq!(owner.continuation(), continuation);
    owner.last_edit = std::time::Instant::now() - Duration::from_secs(1);
    assert!(owner.idle());
}

#[test]
fn original_failure_latches_once_and_rejects_unrequested_completions() {
    let mut owner = OriginalView::default();
    let (signature, _) = request(0.);
    owner.observe(OriginalPurpose::Tool, signature);
    owner.last_edit = std::time::Instant::now() - Duration::from_secs(1);
    let captured = owner.request().unwrap();
    assert!(
        captured
            .completion
            .receive(Err("unrequested".into()))
            .is_none()
    );
    let completion = captured.begin().unwrap();
    assert_eq!(
        completion
            .receive(Err("source unavailable".into()))
            .as_deref(),
        Some("Original view failed: source unavailable")
    );
    assert!(owner.request().is_none());
    assert!(completion.receive(Ok(None)).is_none());
    assert!(owner.failed());
}
