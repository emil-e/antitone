use super::*;
use crate::cancellation::Cancellation;
use std::future::Future;

pub(super) mod query;
#[cfg(test)]
pub(super) mod testing;
#[cfg(test)]
mod tests;

use crate::{evaluation::Context, gpu::submission::Queue};

pub(super) type Gpu<'a> = (&'a Context, &'a Queue);
pub(super) type GpuCapture = (Context, Queue);

pub(super) fn capture_gpu(editor: &Editor) -> Option<GpuCapture> {
    let services = editor.source_services.as_ref()?;
    Some((services.evaluation.clone(), services.queue.clone()))
}

pub(super) fn query_gpu(capture: Option<&GpuCapture>) -> Result<Gpu<'_>, Error> {
    let (context, queue) =
        capture.ok_or_else(|| Error::Gpu("image-query services unavailable".into()))?;
    Ok((context, queue))
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
