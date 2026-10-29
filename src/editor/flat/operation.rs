use super::worker;
use crate::{
    application::UiContext,
    cancellation::Cancellation,
    editor::{Editor, FailureKind, document_services::PreparationServices, source},
    raw::Rect,
};
use futures_channel::oneshot;

#[cfg(test)]
mod tests;

pub(super) fn setup<'scope>(
    request: source::Request,
    services: Option<source::Services>,
    receive: oneshot::Receiver<(u64, Rect)>,
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
        let reference = match reference {
            Ok(reference) => reference,
            Err(error) => {
                fail(&cx, FailureKind::FlatReference, error);
                return;
            }
        };
        cx.update(|editor| {
            let edit = editor.document.flat_edit.as_mut().expect("live Flat setup");
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
            editor.status = if editor.pick_region().is_some() {
                "Proposed illuminated rectangle: adjust edges, then Apply."
            } else {
                "Illuminated area is ambiguous; draw a rectangle around the usable light."
            }
            .into();
        });

        // Apply's document transaction key need not match reference preparation's.
        let Ok((id, region)) = receive.await else {
            return;
        };
        let input = cx.read(|editor| worker::CorrectionInput {
            sources: editor.source_provider(),
            region,
            document: editor.document.capture(),
            selected: editor.document.visit.current_source,
            source: source::Context {
                source: reference.ready.developed.clone(),
                reference: reference.ready.reference.clone(),
                correction: None,
            },
            record: reference.ready.record.clone(),
            correction: editor.document.correction.clone(),
        });
        let result = async {
            let request = worker::estimate(input, cancelled.clone()).await?;
            PreparationServices(services.expect("reference preparation used source services"))
                .prepare(request, cancelled, |_, _| {})
                .await
        }
        .await;
        cx.update(|editor| {
            editor.document_prepared(id, result.map(Box::new));
            editor.acquire_working(&cx);
        });
    })
}

fn fail(cx: &UiContext<'_, Editor>, kind: FailureKind, error: String) {
    cx.update(|editor| {
        editor.finish_flat_edit();
        editor.status = error.clone();
        editor.report_failure(kind, error);
    });
}
