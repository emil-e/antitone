use super::*;
use crate::editor::ui::{DecisionDialog, DialogChoice};

fn message(title: &'static str) -> Message {
    Message {
        severity: Severity::Error,
        title,
        explanation: "Accepted state is unchanged.",
        details: "fixture failure".into(),
    }
}

fn frame(dialogs: &mut Dialogs, context: &egui::Context, events: Vec<egui::Event>) {
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800., 600.),
            )),
            events,
            ..Default::default()
        },
        |ui| dialogs.paint(ui.ctx(), true),
    );
    output.textures_delta.clear();
}

impl Dialogs {
    pub(in crate::editor) fn answer_for_test(&mut self, label: &str) {
        let context = egui::Context::default();
        let mut labels = Vec::new();
        for _ in 0..3 {
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800., 600.),
                    )),
                    ..Default::default()
                },
                |ui| self.paint(ui.ctx(), true),
            );
            output.textures_delta.clear();
            labels = output
                .shapes
                .into_iter()
                .filter_map(|shape| match shape.shape {
                    egui::Shape::Text(text) => Some((
                        text.galley.job.text.clone(),
                        egui::Rect::from_min_size(text.pos, text.galley.size()),
                    )),
                    _ => None,
                })
                .collect::<Vec<_>>();
        }
        let point = labels
            .iter()
            .find(|(text, _)| text == label)
            .unwrap_or_else(|| panic!("dialog control {label:?} not found: {labels:?}"))
            .1
            .center();
        for pressed in [true, false] {
            frame(
                self,
                &context,
                vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }
}

fn enter(pressed: bool) -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn submitting_is_immediate_and_abandoning_the_response_does_not_dismiss_the_message() {
    let mut dialogs = Dialogs::default();
    drop(dialogs.report(message("Caller has gone")));
    assert!(dialogs.is_open());
    let context = egui::Context::default();
    frame(&mut dialogs, &context, vec![]);
    frame(&mut dialogs, &context, vec![]);
    assert_eq!(dialogs.message().unwrap().title, "Caller has gone");
    frame(&mut dialogs, &context, vec![enter(true)]);
    assert!(!dialogs.is_open());
}

#[test]
fn each_notice_requires_its_own_user_acknowledgement_and_completes_only_its_response() {
    let mut dialogs = Dialogs::default();
    let mut first = dialogs.report(message("First"));
    let mut second = dialogs.report(message("Second"));
    assert_eq!(first.try_recv(), Ok(None));
    assert_eq!(second.try_recv(), Ok(None));
    assert_eq!(dialogs.message().unwrap().title, "First");
    let context = egui::Context::default();
    frame(&mut dialogs, &context, vec![]);
    frame(&mut dialogs, &context, vec![]);
    frame(&mut dialogs, &context, vec![enter(true)]);
    assert_eq!(first.try_recv(), Ok(Some(())));
    assert_eq!(second.try_recv(), Ok(None));
    assert_eq!(dialogs.message().unwrap().title, "Second");
    frame(&mut dialogs, &context, vec![enter(false)]);
    frame(&mut dialogs, &context, vec![]);
    assert_eq!(second.try_recv(), Ok(None));
    frame(&mut dialogs, &context, vec![enter(true)]);
    assert_eq!(second.try_recv(), Ok(Some(())));
    assert!(!dialogs.is_open());
}

#[test]
fn a_decision_delivers_its_typed_answer_through_the_same_host_and_notifies_admission_waiters() {
    let mut dialogs = Dialogs::default();
    let mut response = dialogs.show(|context, id, escape| {
        DecisionDialog::new(
            "Change the document?",
            "Accepted state is unchanged until confirmation.",
            "Change",
        )
        .show(context, id, escape)
    });
    let changed = dialogs.changes();
    let mut notification = Box::pin(changed.listen());
    assert_eq!(response.try_recv(), Ok(None));
    let context = egui::Context::default();
    frame(&mut dialogs, &context, vec![]);
    frame(&mut dialogs, &context, vec![]);
    frame(&mut dialogs, &context, vec![enter(true)]);
    assert_eq!(response.try_recv(), Ok(Some(DialogChoice::Cancel)));
    assert!(!dialogs.is_open());
    assert!(
        futures_lite::future::block_on(futures_lite::future::poll_once(&mut notification))
            .is_some()
    );
}

#[test]
fn destroying_the_display_cancels_waiters_without_fabricating_a_user_answer() {
    let mut dialogs = Dialogs::default();
    let mut response = dialogs.report(message("No answer"));
    drop(dialogs);
    assert!(response.try_recv().is_err());
}
