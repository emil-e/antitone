use super::worker;
use crate::{
    application::UiContext,
    cancellation::Cancellation,
    editor::{Editor, FailureKind, source},
};

#[cfg(test)]
mod tests;

pub(super) fn setup<'scope>(
    request: source::Request,
    services: Option<source::Services>,
    owner: std::rc::Weak<std::cell::RefCell<super::FlatEdit>>,
    cx: &UiContext<'scope, Editor>,
) -> Option<async_executor::Task<()>> {
    let cancelled = Cancellation::default();
    let cancel_when_dropped = cancelled.cancel_on_drop();
    cx.spawn(move |cx| async move {
        let _cancel_when_dropped = cancel_when_dropped;
        let reference = async {
            let services = services
                .as_ref()
                .ok_or_else(|| "Reference services unavailable".to_owned())?;
            worker::preview(services.clone(), request, cancelled.clone()).await
        }
        .await;
        let Some(owner) = owner.upgrade() else { return };
        let reference = match reference {
            Ok(reference) => reference,
            Err(error) => {
                fail(&cx, FailureKind::FlatReference, error);
                return;
            }
        };
        cx.update(|editor| {
            let mut edit = owner.borrow_mut();
            edit.region = edit
                .region
                .or(reference.region.map(super::Coverage::rectangle));
            edit.view.ready = Some(reference.ready.clone());
            let activity = edit.activity.take();
            if let Some(activity) = activity
                && editor.tasks.finish(activity)
            {
                editor.busy = false;
            }
            editor.resolve_failure(FailureKind::FlatReference);
            editor.status = if edit.region.is_some() {
                "Proposed illuminated rectangle: adjust edges, then Apply."
            } else {
                "Illuminated area is ambiguous; draw a rectangle around the usable light."
            }
            .into();
        });
    })
}

pub(super) fn correct(
    editor: &mut Editor,
    input: super::CorrectionInput,
    cx: &UiContext<'_, Editor>,
) {
    let before = editor.document.state.project.clone();
    editor.retire_pick_actions();
    editor.end_tool();
    editor.cancel_spatial();
    editor.finish_gamma_edit();
    editor.control_commands.clear();
    let id = editor.begin_document_operation(false);
    editor.history_pending = Some(crate::editor::history::Pending::Edit {
        before: Box::new(before),
        label: "Correction",
        photo: None,
    });
    editor.document_failure = Some(FailureKind::FlatCorrection);
    editor.tasks.begin(crate::editor::Task::Correction(id));
    let services = input.services.clone();
    let cancelled = Cancellation::default();
    let retire = cancelled.cancel_on_drop();
    let Some(task) = cx.spawn(move |cx| async move {
        let _retire = retire;
        let result = async {
            let services = services.ok_or_else(|| "Correction services unavailable".to_owned())?;
            let request = worker::estimate(input, cancelled.clone()).await?;
            crate::editor::document::PreparationServices(services)
                .prepare(request, cancelled, |_, _| {})
                .await
        }
        .await;
        cx.update(|editor| {
            if editor.document_pending == Some(id) {
                editor.document.flat_correction = None;
                editor.document_prepared(id, result.map(Box::new));
                editor.acquire_working(&cx);
            }
        });
    }) else {
        editor.document_prepared(id, Err("Application stopped".into()));
        return;
    };
    editor.document.flat_correction = Some(task);
}

fn fail(cx: &UiContext<'_, Editor>, kind: FailureKind, error: String) {
    cx.update(|editor| {
        editor.end_tool();
        editor.status = error.clone();
        editor.report_failure(kind, error);
    });
}
