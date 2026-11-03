use super::{Display, Editor, UiContext};
use crate::{Error, cancellation::Cancellation, coverage::Coverage, scan::Scan};
use futures_channel::oneshot;

pub(in crate::editor) mod query;

#[cfg(test)]
mod operation_tests;
#[cfg(test)]
pub(in crate::editor) mod testing;
#[cfg(test)]
mod tests;

pub(in crate::editor) fn capture_scan(editor: &Editor) -> Result<Scan, String> {
    let ready = editor
        .document
        .visit
        .canvas
        .ready
        .as_ref()
        .ok_or("No developed source for measurement")?;
    let mut scan = Scan::with_reference(ready.developed.clone(), ready.reference.clone());
    scan.set_flat_field(
        editor
            .document
            .correction
            .as_ref()
            .map(|correction| correction.settings.map.clone()),
    )
    .map_err(|error| error.to_string())?;
    Ok(scan)
}

pub(in crate::editor) fn query_services(
    services: Option<&crate::editor::source::Services>,
) -> Result<&crate::editor::source::Services, Error> {
    services.ok_or_else(|| Error::Gpu("image-query services unavailable".into()))
}

/// Open a picker-owned operation and await its typed result independently of the tool.
pub(in crate::editor) fn pick<'scope, T, F>(
    editor: &mut Editor,
    display: Display,
    measure: F,
    cx: UiContext<'scope, Editor>,
) -> impl std::future::Future<Output = Option<T>> + use<'scope, T, F>
where
    T: Send + 'static,
    F: FnOnce(Coverage, Cancellation) -> Result<T, Error> + Send + 'static,
{
    let (mut reply, receive) = oneshot::channel();
    let previous = editor.document.visit.canvas.last_layout;
    if editor.prepare_tool() {
        editor.retire_pick_actions();
        let (submit, selected) = oneshot::channel();
        let id = editor.request_id();
        #[cfg(test)]
        let gate = editor.document.visit.picker.measurement_gate.clone();
        // The task retains work and communication endpoints, never its tool/UI owner.
        let task = cx.spawn(move |cx| async move {
            let measured = async {
                let region = selected.await.ok()?;
                Some(
                    crate::background::run(move |cancelled| {
                        #[cfg(test)]
                        if let Some(gate) = &gate
                            && !gate.is_after_work()
                        {
                            gate.wait(cancelled.clone())?;
                        }
                        let result =
                            measure(region, cancelled.clone()).map_err(|error| error.to_string());
                        #[cfg(test)]
                        if let Some(gate) = &gate
                            && gate.is_after_work()
                        {
                            gate.wait(cancelled)?;
                        }
                        result
                    })
                    .await
                    .map_err(|_| "Measurement worker panicked".to_owned())
                    .and_then(|result| result),
                )
            };
            let stopped = async {
                reply.cancellation().await;
                None
            };
            let result = futures_lite::future::or(stopped, measured).await;
            let value = cx
                .update(|editor| {
                    if let Some(Err(error)) = &result {
                        editor.status = error.clone();
                    }
                    editor.end_tool();
                    result.and_then(Result::ok)
                })
                .flatten();
            if let Some(value) = value {
                let _ = reply.send(value);
            }
        });
        if let Some(task) = task {
            super::install(
                editor,
                display,
                previous,
                Some(submit),
                Some(id),
                Some(task),
            );
        }
    }
    async move { receive.await.ok() }
}
