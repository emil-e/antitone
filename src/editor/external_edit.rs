//! Document-scoped editing on behalf of another application.
use super::*;
use crate::cancellation::Cancellation;

pub struct Request(Box<dyn Input>);

impl Request {
    pub(super) fn new(input: impl Input + 'static) -> Self {
        Self(Box::new(input))
    }
}

/// Resolves host access and editing metadata before application composition installs the document.
pub(super) trait Input: Send {
    fn presentation(&self) -> Presentation;
    fn prepare(
        self: Box<Self>,
        cancelled: &Cancellation,
        progress: &mut dyn FnMut(usize, usize),
    ) -> Result<Opened, Error>;
}

#[derive(Clone, Copy)]
pub(super) struct Presentation {
    pub action: &'static str,
    pub opening: &'static str,
    pub working: &'static str,
    pub completed: &'static str,
}

/// Host output planning, persistence and publication; shared output executes pixels.
pub(super) trait Destination: Send + Sync {
    fn presentation(&self) -> Presentation;
    fn output(&self, document: &Document, target: OutputTarget) -> Result<output::Options, Error>;
    fn save(&self, document: &Document) -> Result<Document, Error>;
    /// Observe a successful ordinary or return save; this performs no persistence.
    fn saved(&self, _path: &std::path::Path) {}
    fn publish(&self, document: &Document, renders: &[Rendered]) -> Result<(), Error>;
}

pub(super) struct Rendered {
    pub source: SourceId,
    pub path: PathBuf,
}
pub(super) struct Opened {
    pub document: Document,
    pub session: Session,
}

pub(super) struct Session {
    requested: Arc<[SourceId]>,
    destination: Arc<dyn Destination>,
}

impl Session {
    pub fn new(
        document: &Document,
        requested: Vec<SourceId>,
        destination: Arc<dyn Destination>,
    ) -> Result<Self, Error> {
        if requested.is_empty()
            || requested
                .iter()
                .any(|id| !document.project.order.contains(id))
            || requested
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                != requested.len()
        {
            return Err(Error::Project(
                "external edit requires distinct ordered sources".into(),
            ));
        }
        Ok(Self {
            requested: requested.into(),
            destination,
        })
    }
    pub fn selected(&self) -> SourceId {
        self.requested[0]
    }
}

#[derive(Default)]
pub(super) struct State {
    pub session: Option<Arc<Session>>,
    task: Option<async_executor::Task<()>>,
    pub cancelled: Cancellation,
    returned: bool,
}

impl State {
    pub fn presentation(&self) -> Option<Presentation> {
        self.session
            .as_ref()
            .map(|session| session.destination.presentation())
    }
    pub fn pending(&self) -> bool {
        self.task.is_some()
    }
    pub fn saved(&self, path: &std::path::Path) {
        if let Some(session) = &self.session {
            session.destination.saved(path);
        }
    }
    fn owns(&self, session: &Arc<Session>) -> bool {
        self.session
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, session))
    }
}

impl Editor {
    pub fn open_external(&mut self, request: Request, cx: &UiContext<'_, Self>) {
        let activity = self
            .activities
            .start(ActivityOptions::new(request.0.presentation().opening).modal());
        let progress = activity.reporter();
        let cancelled = Cancellation::default();
        let stop = cancelled.clone();
        let retire = cancelled.cancel_on_drop();
        self.document.external_edit.cancelled = stop.clone();
        self.document.external_edit.task = cx.spawn(move |cx| async move {
            let _retire = retire;
            let result = crate::background::run(move |_| {
                request
                    .0
                    .prepare(&stop, &mut |done, total| progress.progress(done, total))
            })
            .await
            .map_err(|_| "External edit reader panicked".to_owned())
            .and_then(|result| result.map_err(|error| error.to_string()));
            drop(activity);
            cx.update(|editor| match result {
                Ok(opened) => editor.accept_external(opened, &cx),
                Err(error) => {
                    editor.document.external_edit.task = None;
                    editor.external_failed(error);
                }
            });
        });
    }

    pub(super) fn external_return_button(&self, ui: &mut egui::Ui) -> bool {
        self.document
            .external_edit
            .presentation()
            .is_some_and(|presentation| {
                ui::TextAction::new(presentation.action)
                    .enabled(self.external_return_available())
                    .show(ui)
                    .clicked()
            })
    }

    pub(super) fn queue_external_return(&mut self, cx: &UiContext<'_, Self>) {
        if let Some(task) = cx.spawn(|cx| async move {
            cx.update(|editor| editor.return_external(&cx));
        }) {
            task.detach();
        }
    }

    pub(super) fn external_return_available(&self) -> bool {
        self.document.external_edit.session.is_some()
            && !self.document.external_edit.pending()
            && !self.document.external_edit.returned
            && !self.workspace_input.is_blocked()
            && !self.editing()
            && !self.allow_close
            && self.document.project.input.ready()
    }

    pub(super) fn return_external(&mut self, cx: &UiContext<'_, Self>) {
        if !self.external_return_available() {
            return;
        }
        let session = self
            .document
            .external_edit
            .session
            .as_ref()
            .unwrap()
            .clone();
        let presentation = session.destination.presentation();
        let captured = (|| {
            if session
                .requested
                .iter()
                .any(|id| !self.document.project.order.contains(id))
            {
                return Err(Error::Project(
                    "a requested photograph was removed from the roll".into(),
                ));
            }
            let options = session
                .destination
                .output(&self.document, self.document.target())?;
            Ok::<_, Error>((
                self.document.capture(),
                self.capture_output(&options, session.requested.to_vec())?,
            ))
        })();
        let (document, output) = match captured {
            Ok(captured) => captured,
            Err(error) => {
                self.external_failed(error.to_string());
                return;
            }
        };
        let Some(services) = self.output_services.clone() else {
            self.external_failed("Output services unavailable".into());
            return;
        };
        let activity = self
            .activities
            .start(ActivityOptions::new(presentation.working).modal());
        let cancelled = Cancellation::default();
        self.document.external_edit.cancelled = cancelled.clone();
        let retire = cancelled.cancel_on_drop();
        self.document.external_edit.task = cx.spawn(move |cx| async move {
            let _retire = retire;
            let result = async {
                let host = session.destination.clone();
                let stop = cancelled.clone();
                let document = crate::background::run(move |retired| {
                    if retired.is_cancelled() || stop.is_cancelled() {
                        return Err(Error::Cancelled);
                    }
                    host.save(&document)
                })
                .await
                .map_err(|_| "Project save worker panicked".to_owned())?
                .map_err(|error| error.to_string())?;
                cx.update(|editor| {
                    if editor.document.external_edit.owns(&session) {
                        editor.document.accept_save(document.clone());
                    }
                });
                let summary = services
                    .export_batch(output, cancelled.clone(), |done, total| {
                        activity.progress(done, total)
                    })
                    .await;
                if summary.failed() != 0
                    || summary.stopped
                    || summary.exported() != session.requested.len()
                {
                    let errors = summary
                        .outcomes
                        .iter()
                        .filter_map(|outcome| match outcome {
                            output::Outcome::Failed { error, .. } => Some(error.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("; ");
                    return Err(format!(
                        "The roll was saved, but return failed: {}. {errors}",
                        summary.text()
                    ));
                }
                let renders = summary
                    .outcomes
                    .into_iter()
                    .zip(session.requested.iter())
                    .filter_map(|(outcome, source)| match outcome {
                        output::Outcome::Exported(path) => Some(Rendered {
                            source: *source,
                            path,
                        }),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                let host = session.destination.clone();
                crate::background::run(move |retired| {
                    if retired.is_cancelled() || cancelled.is_cancelled() {
                        return Err(Error::Cancelled);
                    }
                    host.publish(&document, &renders)
                })
                .await
                .map_err(|_| "Return publication worker panicked".to_owned())?
                .map_err(|error| format!("Images rendered, but return publication failed: {error}"))
            }
            .await;
            drop(activity);
            cx.update(|editor| {
                if !editor.document.external_edit.owns(&session) {
                    return;
                }
                editor.document.external_edit.task = None;
                match result {
                    Ok(()) => {
                        editor.document.external_edit.returned = true;
                        editor.status = presentation.completed.into();
                        editor.complete_external();
                    }
                    Err(error) => editor.external_failed(error),
                }
            });
        });
    }

    fn external_failed(&mut self, details: String) {
        log::error!(error = details.as_str(); "external_edit_failed");
        self.status = details.clone();
        drop(self.dialogs.report(dialogs::Message {
            severity: ui::Severity::Error,
            title: "Could not complete external edit",
            explanation: "Review the details before retrying. Any successfully saved projects or rendered images remain on disk.",
            details,
        }));
    }
}

#[cfg(test)]
mod tests;
