use super::*;
use crate::cancellation::Cancellation;
use std::future::Future;

pub(super) mod query;
#[cfg(test)]
pub(super) mod testing;
#[cfg(test)]
mod tests;

#[cfg(test)]
pub(super) fn capture_scan(editor: &Editor) -> Result<Scan, String> {
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

pub(super) fn capture_input(
    editor: &Editor,
) -> Result<impl Future<Output = Result<Scan, String>> + use<>, String> {
    let input = editor.capture_source_context()?;
    Ok(async move {
        let context = input.await?;
        let mut scan = Scan::with_reference(context.source, context.reference);
        scan.set_flat_field(
            context
                .correction
                .map(|correction| correction.settings.map.clone()),
        )
        .map_err(|error| error.to_string())?;
        Ok(scan)
    })
}

pub(super) fn query_services(
    services: Option<&crate::editor::source::Services>,
) -> Result<&crate::editor::source::Services, Error> {
    services.ok_or_else(|| Error::Gpu("image-query services unavailable".into()))
}

/// Await photographic work with cancellable measurement feedback, independently of selection.
pub(super) async fn run<T>(
    cx: &UiContext<'_, Editor>,
    label: &str,
    work: impl Future<Output = Result<T, String>>,
) -> Option<T> {
    let cancelled = Cancellation::default();
    let activity = cx.read(|editor| {
        editor.activities.start(
            ActivityOptions::new(format!("Measuring {label}…"))
                .modal()
                .cancellable(
                    "Cancel measurement",
                    "Cancelling measurement…",
                    cancelled.clone(),
                ),
        )
    });
    let stopped = async {
        activity.cancelled().await;
        None
    };
    let result = futures_lite::future::or(stopped, async { Some(work.await) }).await;
    drop(activity);
    if let Some(Err(error)) = &result {
        cx.update(|editor| editor.status = error.clone());
    }
    result.and_then(Result::ok)
}

/// Execute the captured synchronous measurement off the UI thread.
pub(super) async fn query<T: Send + 'static>(
    _cx: &UiContext<'_, Editor>,
    work: impl FnOnce(Cancellation) -> Result<T, Error> + Send + 'static,
) -> Result<T, String> {
    #[cfg(test)]
    let gate = _cx.read(|editor| editor.document.visit.measurement_gate.clone());
    crate::background::run(move |cancelled| {
        #[cfg(test)]
        if let Some(gate) = &gate
            && !gate.is_after_work()
        {
            gate.wait(cancelled.clone())?;
        }
        let result = work(cancelled.clone()).map_err(|error| error.to_string());
        #[cfg(test)]
        if let Some(gate) = &gate
            && gate.is_after_work()
        {
            gate.wait(cancelled)?;
        }
        result
    })
    .await
    .map_err(|_| "Measurement worker panicked".to_owned())?
}

#[cfg(test)]
impl Editor {
    pub(super) fn measurement_pending(&self) -> bool {
        self.activities.any(|view| {
            view.title.starts_with("Measuring ") || view.title == "Cancelling measurement…"
        })
    }
}
