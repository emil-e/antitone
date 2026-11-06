use super::presentation::{FeedbackDialog, Severity};
use futures_channel::oneshot;
use std::{collections::VecDeque, sync::Arc};

#[derive(Clone, Debug)]
pub(super) struct Message {
    pub severity: Severity,
    pub title: &'static str,
    pub explanation: &'static str,
    pub details: String,
}

impl Message {
    fn presentation(&self) -> FeedbackDialog<'_> {
        FeedbackDialog {
            severity: self.severity,
            title: self.title,
            explanation: self.explanation,
            details: &self.details,
        }
    }
}

type Paint = Box<dyn FnMut(&egui::Context, bool) -> bool>;

struct Pending {
    paint: Paint,
    #[cfg(test)]
    message: Option<Message>,
}

/// Owns submitted presentation until the user answers; callers own the consequences.
#[derive(Default)]
pub(super) struct Dialogs {
    pending: VecDeque<Pending>,
    next: u64,
    changed: Arc<event_listener::Event>,
}

impl Dialogs {
    /// Submission is immediate. Dropping the response only abandons the continuation.
    pub fn show<R: 'static>(
        &mut self,
        mut present: impl FnMut(&egui::Context, egui::Id, bool) -> Option<R> + 'static,
    ) -> oneshot::Receiver<R> {
        let (send, receive) = oneshot::channel();
        let mut send = Some(send);
        let id = egui::Id::new("dialog").with(self.next);
        self.next += 1;
        self.pending.push_back(Pending {
            paint: Box::new(move |context, escape| {
                let Some(answer) = present(context, id, escape) else {
                    return false;
                };
                let _ = send.take().unwrap().send(answer);
                true
            }),
            #[cfg(test)]
            message: None,
        });
        self.changed.notify(usize::MAX);
        receive
    }

    pub fn report(&mut self, message: Message) -> oneshot::Receiver<()> {
        log::warn!(title = message.title, error = message.details.as_str(); "operation_failed");
        #[cfg(test)]
        let observed = message.clone();
        let response = self.show(move |context, id, escape| {
            message
                .presentation()
                .show(context, id, escape)
                .then_some(())
        });
        #[cfg(test)]
        {
            self.pending.back_mut().unwrap().message = Some(observed);
        }
        response
    }

    pub fn is_open(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn changes(&self) -> Arc<event_listener::Event> {
        self.changed.clone()
    }

    pub fn paint(&mut self, context: &egui::Context, escape: bool) {
        if self
            .pending
            .front_mut()
            .is_some_and(|dialog| (dialog.paint)(context, escape))
        {
            self.pending.pop_front();
            self.changed.notify(usize::MAX);
            context.request_repaint();
        }
    }

    #[cfg(test)]
    pub fn message(&self) -> Option<Message> {
        self.pending.front()?.message.clone()
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod workflow_tests;
