use super::{dust, source};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[derive(Clone)]
pub(super) struct Services;

impl Services {
    pub fn new() -> Self {
        Self
    }

    pub async fn search(
        self,
        request: dust::SearchRequest,
    ) -> Result<crate::dust_repair::SearchOutcome, String> {
        crate::background::run(move || {
            let stopped = || request.cancelled.is_cancelled();
            if stopped() {
                return Ok(crate::dust_repair::SearchOutcome::Cancelled);
            }
            let scan = request.context.scan()?;
            let source = crate::scan::ImageSource::new(&scan, request.input)?;
            crate::repair_source::search(
                &source,
                &request.support,
                request.previous.as_ref().map(|repair| repair.offset),
                &mut crate::dust_repair::Execution {
                    cancelled: &stopped,
                },
            )
        })
        .await
        .map_err(|_| "Dust search worker panicked".to_owned())?
        .map_err(|error: crate::Error| error.to_string())
    }

    pub async fn evaluate(
        self,
        request: dust::Request,
        input: source::Context,
        source_epoch: Arc<AtomicU64>,
    ) -> Result<Option<dust::Response>, String> {
        crate::background::run(move || {
            let cancelled = || {
                if source_epoch.load(Ordering::Relaxed) != request.loaded {
                    request.cancelled.store(true, Ordering::Relaxed);
                }
                request.cancelled.load(Ordering::Relaxed)
            };
            if cancelled() {
                return Ok(None);
            }
            let result = input
                .scan()
                .and_then(|scan| dust::evaluate_with(&scan, &request, &cancelled));
            if cancelled() { Ok(None) } else { result }
        })
        .await
        .map_err(|_| "Dust evaluation worker panicked".to_owned())?
        .map_err(|error: crate::Error| error.to_string())
    }
}
