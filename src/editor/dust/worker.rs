use super::{Request, Response, evaluate_with, search, source};
use crate::{cancellation::Cancellation, dust_repair::SearchOutcome};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[cfg(test)]
pub(super) mod testing;

pub(super) async fn search(input: search::Input) -> Result<SearchOutcome, String> {
    crate::background::run(move |cancelled| {
        #[cfg(test)]
        let input = {
            let mut input = input;
            if let Some(worker) = input.worker.take() {
                return worker.run(input, cancelled);
            }
            input
        };
        search_pixels(input, &cancelled).map_err(|error| error.to_string())
    })
    .await
    .map_err(|_| "Dust search worker panicked".to_owned())?
}

fn search_pixels(
    input: search::Input,
    cancelled: &Cancellation,
) -> Result<SearchOutcome, crate::Error> {
    let stopped = || cancelled.is_cancelled();
    if stopped() {
        return Ok(SearchOutcome::Cancelled);
    }
    let scan = input.context.scan()?;
    let source = crate::scan::ImageSource::new(&scan, input.input)?;
    crate::repair_source::search(
        &source,
        &input.support,
        input.previous_offset,
        &mut crate::dust_repair::Execution {
            cancelled: &stopped,
        },
    )
}

pub(super) async fn evaluate(
    request: Request,
    input: source::Context,
    source_epoch: Arc<AtomicU64>,
) -> Result<Option<Response>, String> {
    crate::background::run(move |retired| {
        let cancelled = || {
            if retired.is_cancelled() || source_epoch.load(Ordering::Relaxed) != request.loaded {
                request.cancelled.store(true, Ordering::Relaxed);
            }
            request.cancelled.load(Ordering::Relaxed)
        };
        if cancelled() {
            return Ok(None);
        }
        let result = input
            .scan()
            .and_then(|scan| evaluate_with(&scan, &request, &cancelled));
        if cancelled() { Ok(None) } else { result }
    })
    .await
    .map_err(|_| "Dust evaluation worker panicked".to_owned())?
    .map_err(|error: crate::Error| error.to_string())
}
