#[cfg(test)]
mod publication_tests;
#[cfg(test)]
mod tests;

use super::{ActivityOptions, Editor, UiContext, dialogs, document::Document};
use std::path::PathBuf;

/// The document owns publication of its written baseline independently of a caller's next action.
#[derive(Default)]
pub(super) struct State {
    task: Option<async_executor::Task<()>>,
}

impl State {
    pub fn pending(&self) -> bool {
        self.task.is_some()
    }
}

pub(super) struct Write {
    pub document: Document,
    pub path: PathBuf,
    pub policy: crate::project::Save,
}

impl Write {
    pub fn execute(
        self,
        destination: Option<&dyn super::external_edit::Destination>,
    ) -> Result<(Document, Result<(), crate::Error>), crate::Error> {
        let saved = self.document.save(&self.path, self.policy)?;
        let notification = destination.map_or(Ok(()), |host| host.saved(&saved));
        Ok((saved, notification))
    }
}

impl Editor {
    pub(super) fn start_save(
        &mut self,
        write: Write,
        cx: &UiContext<'_, Self>,
    ) -> Option<futures_channel::oneshot::Receiver<bool>> {
        let (send, receive) = futures_channel::oneshot::channel();
        if self.document.write.pending() || self.editing() {
            self.status = "Finish or cancel the current edit before saving".into();
            return None;
        }
        self.document.history.finish_group();
        let activity = self
            .activities
            .start(ActivityOptions::new("Saving project…").modal());
        let destination = self.document.external_edit.destination();
        self.document.write.task = cx.spawn(move |cx| async move {
            let result = crate::background::run(move |_| write.execute(destination.as_deref()))
                .await
                .map_err(|_| "Document write worker panicked".to_owned())
                .and_then(|result| result.map_err(|error| error.to_string()));
            drop(activity);
            let succeeded = result.is_ok();
            cx.update(|editor| {
                editor.document.write = State::default();
                match result {
                    Ok((document, notification)) => {
                        editor.document.accept_save(document);
                        editor.status = "Project saved".into();
                        if let Err(error) = notification {
                            editor.external_failed(format!(
                                "The project was saved, but could not be linked: {error}"
                            ));
                        }
                    }
                    Err(error) => {
                        editor.status = format!("Save failed: {error}");
                        editor.save_failed(error);
                    }
                }
            });
            let _ = send.send(succeeded);
        });
        self.document.write.pending().then_some(receive)
    }

    pub(super) fn save_failed(&mut self, details: String) {
        drop(self.dialogs.report(dialogs::Message {
            severity: super::ui::Severity::Error,
            title: "Could not save project",
            explanation: "Your edits are still in memory and the destination file was not replaced. Review the details, then try saving again.",
            details,
        }));
    }
}
