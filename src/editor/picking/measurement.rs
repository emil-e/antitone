use super::{Display, Editor, Selection, UiContext};
use crate::{Error, cancellation::Cancellation, coverage::Coverage, scan::Scan};
use futures_channel::oneshot;
use std::rc::Rc;

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

struct Retire<'scope> {
    selection: Rc<Selection>,
    cx: UiContext<'scope, Editor>,
    armed: bool,
}

impl Drop for Retire<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        self.selection.cancelled.cancel();
        self.selection.reply.borrow_mut().take();
        let selection = self.selection.clone();
        // A caller may drop its future while the UI state is borrowed. Cleanup
        // runs on the host executor and retains no pixel or document lease.
        if let Some(cleanup) = self.cx.spawn(move |cx| async move {
            cx.update(|editor| editor.finish_selection(&selection));
        }) {
            cleanup.detach();
        }
    }
}

/// Open the interaction now; await selection and measurement without retaining the Editor borrow.
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
    let (reply, receive) = oneshot::channel();
    let selection = Rc::new(Selection {
        id: editor.request_id(),
        reply: std::cell::RefCell::new(Some(reply)),
        cancelled: Cancellation::default(),
    });
    let prepared = editor
        .install_picker(display, Some(selection.clone()))
        .map(|_| Retire {
            selection: selection.clone(),
            cx: cx.clone(),
            armed: true,
        });
    #[cfg(test)]
    let gate = editor.document.visit.picking.measurement_gate.clone();
    async move {
        let mut retire = prepared?;
        if selection.cancelled.is_cancelled() {
            return None;
        }
        let region = receive.await.ok()?;
        if selection.cancelled.is_cancelled() {
            return None;
        }
        let measured = async {
            crate::background::run(move |cancelled| {
                #[cfg(test)]
                if let Some(gate) = gate {
                    gate.wait(cancelled.clone())?;
                }
                measure(region, cancelled).map_err(|error| error.to_string())
            })
            .await
            .map_err(|_| "Measurement worker panicked".to_owned())
            .and_then(|result| result)
        };
        let stopped = async {
            selection.cancelled.wait().await;
            None
        };
        let result = futures_lite::future::or(async { Some(measured.await) }, stopped).await?;
        if selection.cancelled.is_cancelled() {
            return None;
        }
        cx.update(|editor| {
            if let Err(error) = &result {
                editor.status = error.clone();
            }
            editor.finish_selection(&selection);
        })?;
        retire.armed = false;
        result.ok()
    }
}
