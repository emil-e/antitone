use super::{Display, Editor, Selection, UiContext};
use crate::{Error, cancellation::Cancellation, coverage::Coverage, scan::Scan};
use futures_channel::oneshot;
use std::rc::Rc;

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

/// Await a region selection and its measurement; cancellation and reported failures yield no value.
pub(in crate::editor) async fn pick<T, F>(
    display: Display,
    measure: F,
    cx: UiContext<'_, Editor>,
) -> Option<T>
where
    T: Send + 'static,
    F: FnOnce(Coverage, Cancellation) -> Result<T, Error> + Send + 'static,
{
    let (selection, receive) = cx.update(|editor| {
        if !matches!(
            editor.document.tool,
            Some(super::super::tool::Tool::Pick(_))
        ) {
            if !editor.prepare_tool() {
                return None;
            }
            editor.document.tool = Some(super::super::tool::Tool::Pick(Vec::new()));
        }
        if editor.document.visit.picking.selection.is_some() {
            editor.cancel_picker();
        }
        editor.show_picker(display);
        let (reply, receive) = oneshot::channel();
        let selection = Rc::new(Selection {
            id: editor.request_id(),
            reply: std::cell::RefCell::new(Some(reply)),
            cancelled: Cancellation::default(),
        });
        editor.document.visit.picking.selection = Some(selection.clone());
        Some((selection, receive))
    })??;
    let mut retire = Retire {
        selection: selection.clone(),
        cx: cx.clone(),
        armed: true,
    };
    #[cfg(test)]
    let gate = cx.read(|editor| editor.document.visit.picking.measurement_gate.clone());
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
