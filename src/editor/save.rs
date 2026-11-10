#[cfg(test)]
mod tests;

use super::{
    ActivityOptions, Editor, UiContext, dialogs,
    document::{Document, Write},
};

#[derive(Default)]

pub(super) struct State {
    task: Option<async_executor::Task<()>>,
    response: Option<futures_channel::oneshot::Sender<bool>>,
}

impl Editor {
    pub(super) fn start_save(
        &mut self,
        write: Write,
        cx: &UiContext<'_, Self>,
    ) -> futures_channel::oneshot::Receiver<bool> {
        let (send, receive) = futures_channel::oneshot::channel();
        if self.dust_search_pending() || self.editing() {
            self.status = "Finish or cancel the current edit before saving".into();
            let _ = send.send(false);
            return receive;
        }
        if self.picking() {
            self.end_tool();
        }
        self.document.history.finish_group();
        self.history_pending = None;
        let id = self.begin_document_operation();
        self.document.write.response = Some(send);
        let activity = self
            .activities
            .start(ActivityOptions::new("Saving project…").modal());

        let device = self
            .source_services
            .as_ref()
            .map(|source| source.device.clone());
        let Some(task) = cx.spawn(move |cx| async move {
            let result = match device {
                Some(device) => crate::background::run(move |_| write.save(&device))
                    .await
                    .map_err(|_| "Document write worker panicked".to_owned())
                    .and_then(|result| result.map(Box::new).map_err(|error| error.to_string())),
                None => Err("Document services unavailable".into()),
            };
            drop(activity);
            cx.update(|editor| {
                editor.save_result(id, result, &cx);
                // The close continuation resumes outside this transaction.
                if !editor.files.closing() {
                    editor.acquire_working(&cx);
                }
            });
        }) else {
            self.save_result(id, Err("Application stopped".into()), cx);
            return receive;
        };
        self.document.write.task = Some(task);
        receive
    }

    pub(super) fn save_result(
        &mut self,
        id: u64,
        result: Result<Box<Document>, String>,
        _cx: &UiContext<'_, Self>,
    ) {
        if self.document_pending != Some(id) {
            return;
        }
        let response = self.document.write.response.take();
        self.document.write = State::default();
        let succeeded = result.is_ok();
        self.source_epoch
            .store(self.id, std::sync::atomic::Ordering::Relaxed);

        self.document_pending = None;

        self.resume_source_after_document();
        match result {
            Ok(document) => {
                log::info!(request = id; "project_saved");
                self.document.accept_save(*document);
                self.status = "Project saved".into();
            }
            Err(error) => {
                self.status = format!("Save failed: {error}");

                self.save_failed(error);
            }
        }
        if let Some(response) = response {
            let _ = response.send(succeeded);
        }
        if self.document_pending.is_none() && self.document.spatial_preparing.is_some() {
            self.status = "Preparing film and picture regions…".into();
        } else {
            self.resume_region_actions();
        }
    }
}

impl Editor {
    pub(super) fn save_failed(&mut self, details: String) {
        drop(self.dialogs.report(dialogs::Message {
            severity: super::presentation::Severity::Error,
            title: "Could not save project",
            explanation: "Your edits are still in memory and the destination file was not replaced. Review the details, then try saving again.",
            details,
        }));
    }
}
