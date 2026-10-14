use super::*;
fn state() -> (Editor, mpsc::Sender<Message>, mpsc::Receiver<Command>) {
    let (mut editor, send, commands) = crate::editor::tests::fixture();
    crate::editor::tests::add_ready_fixture(&mut editor);
    (editor, send, commands)
}
fn request(commands: &mpsc::Receiver<Command>) -> (u64, u64) {
    match commands.try_recv().unwrap() {
        Command::AnalysisSupport(request, loaded) => (request, loaded),
        _ => panic!("expected lazy support request"),
    }
}
#[test]
fn picture_suggestion_is_worker_demand_and_is_transient_until_applied() {
    let (mut editor, send, commands) = state();
    let before = editor.document.project.clone();
    let support = Rect {
        x: 100,
        y: 100,
        width: 600,
        height: 400,
    };
    editor.begin_pick(Pick::Analysis);
    let (request, loaded) = request(&commands);
    assert!(!editor.busy && editor.region.is_none());
    send.send(Message::AnalysisSupport(request, loaded, Ok(Some(support))))
        .unwrap();
    editor.receive();
    assert_eq!(
        editor.region,
        Some(Coverage::rectangle(analysis::suggest(support)))
    );
    assert_eq!(editor.document.project, before);
    editor.finish_pick();
    editor.begin_pick(Pick::Analysis);
    assert!(
        commands.try_recv().is_err(),
        "same ready context already has its suggestion"
    );
    assert_eq!(editor.document.project, before);
}
#[test]
fn delayed_suggestion_cannot_overwrite_drawing_cancelled_picker_or_reopened_picker() {
    for action in ["draw", "cancel", "reopen", "source", "document"] {
        let (mut editor, send, commands) = state();
        editor.begin_pick(Pick::Analysis);
        let (old, loaded) = request(&commands);
        let drawn = Coverage::rectangle(Rect {
            x: 200,
            y: 200,
            width: 200,
            height: 100,
        });
        match action {
            "draw" => editor.region = Some(drawn),
            "cancel" => editor.finish_pick(),
            "reopen" => {
                editor.finish_pick();
                editor.begin_pick(Pick::Analysis);
                let (new, _) = request(&commands);
                assert_ne!(old, new);
            }
            "source" => editor.load("another-source.ARW".into()),
            "document" => editor.document_pending = Some(999),
            _ => unreachable!(),
        }
        let before = editor.document.project.clone();
        send.send(Message::AnalysisSupport(
            old,
            loaded,
            Ok(Some(Rect {
                x: 100,
                y: 100,
                width: 600,
                height: 400,
            })),
        ))
        .unwrap();
        editor.receive();
        assert_eq!(editor.document.project, before);
        if action == "draw" {
            assert_eq!(editor.region, Some(drawn));
        } else {
            assert!(editor.region.is_none());
            assert_eq!(editor.ready.as_ref().and_then(|r| r.analysis_support), None);
        }
    }
}
#[test]
fn accepted_geometry_and_known_missing_support_require_no_lazy_work_and_reset_keeps_roll_order() {
    for known in [
        Some(None),
        Some(Some(Rect {
            x: 100,
            y: 100,
            width: 600,
            height: 400,
        })),
    ] {
        let (mut editor, _, commands) = state();
        editor.ready.as_mut().unwrap().analysis_support = known;
        editor.begin_pick(Pick::Analysis);
        assert!(commands.try_recv().is_err());
        editor.finish_pick();
        editor.reset_analysis_area();
        assert!(!editor.analysis_request().unwrap().local_support);
    }
    let (mut editor, _, commands) = state();
    assert!(editor.analysis_request().unwrap().local_support);
    editor.document.project.analysis_area = Some([100, 100, 500, 300]);
    editor.begin_pick(Pick::Analysis);
    assert!(commands.try_recv().is_err());
    assert_eq!(
        editor.region,
        Some(Coverage::rectangle(Rect {
            x: 100,
            y: 100,
            width: 500,
            height: 300
        }))
    );
}
#[test]
fn support_failure_does_not_change_processing_or_poison_later_demand() {
    let (mut editor, send, commands) = state();
    let before = editor.document.project.clone();
    editor.begin_pick(Pick::Analysis);
    let (request, loaded) = request(&commands);
    send.send(Message::AnalysisSupport(
        request,
        loaded,
        Err("support unavailable".into()),
    ))
    .unwrap();
    editor.receive();
    assert!(editor.region.is_none());
    assert_eq!(editor.document.project, before);
    editor.finish_pick();
    editor.begin_pick(Pick::Analysis);
    assert!(commands.try_recv().is_ok());
}
