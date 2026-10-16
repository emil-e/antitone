//! One CPU geometry producer; the calling worker exclusively owns GPU/cache/file operations.
use super::*;

const STACK_BYTES: usize = 2 * 1024 * 1024;

pub(super) fn encode<R: TileReader>(
    composition: Composition,
    target: OutputTarget,
    metadata: ProcessingMetadata,
    destination: (&Path, Overwrite),
    read: &mut R,
) -> Result<(), Error> {
    encode_planned(
        composition,
        target,
        metadata,
        destination,
        read,
        move |origin, extent| plan(composition, origin, extent, true),
    )
}

fn encode_planned<R: TileReader>(
    composition: Composition,
    target: OutputTarget,
    metadata: ProcessingMetadata,
    destination: (&Path, Overwrite),
    read: &mut R,
    mut build: impl FnMut([u32; 2], [u32; 2]) -> Result<ReconstructionPlan, Error> + Send,
) -> Result<(), Error> {
    let [width, height] = composition.size();
    let order = if composition.is_exact() {
        export::BlockOrder::RowMajor
    } else {
        export::BlockOrder::Spatial2x2
    };
    #[cfg(test)]
    let metrics = diagnostics::Metrics::default();
    #[cfg(test)]
    let producer_metrics = metrics.clone();
    #[cfg(test)]
    let mut wait = std::time::Duration::ZERO;

    std::thread::scope(|scope| {
        // With no queue, the consumer and producer can own at most one plan each.
        let (tx, rx) = std::sync::mpsc::sync_channel(0);
        let producer = std::thread::Builder::new()
            .name("export-geometry".into())
            .stack_size(STACK_BYTES)
            .spawn_scoped(scope, move || {
                #[cfg(test)]
                let (mut elapsed, mut blocks) = (std::time::Duration::ZERO, 0usize);
                for (origin, extent) in export::ordered_blocks(ImageSize { width, height }, order) {
                    #[cfg(test)]
                    let start = std::time::Instant::now();
                    let plan = build(origin, extent).and_then(ReconstructionPlan::admitted);
                    #[cfg(test)]
                    {
                        elapsed += start.elapsed();
                        blocks += 1;
                    }
                    let failed = plan.is_err();
                    let message = Message {
                        origin,
                        extent,
                        #[cfg(test)]
                        _charge: producer_metrics
                            .charge(plan.as_ref().map_or(0, |p| p.payload_bytes().unwrap())),
                        plan,
                    };
                    // A stopped/failed consumer disconnects even if this send is blocked.
                    if tx.send(message).is_err() || failed {
                        break;
                    }
                }
                #[cfg(test)]
                {
                    (elapsed, blocks)
                }
            })
            .map_err(|e| Error::Export(format!("cannot start export geometry worker: {e}")))?;

        let result = encode_with(
            composition,
            Format::Tiff(target),
            metadata,
            destination,
            |origin, extent| {
                #[cfg(test)]
                let start = std::time::Instant::now();
                let message = rx
                    .recv()
                    .map_err(|_| Error::Export("export geometry worker stopped".into()))?;
                #[cfg(test)]
                {
                    wait += start.elapsed();
                }
                if (message.origin, message.extent) != (origin, extent) {
                    return Err(Error::Export("export geometry block order mismatch".into()));
                }
                let result = message.plan.and_then(|plan| complete(plan, read));
                #[cfg(test)]
                drop(message._charge);
                result
            },
        );
        // Drop before joining: the producer may be waiting to deliver a plan after an encoder error.
        // On consumer panic, local destruction also disconnects before the scope waits for the thread.
        drop(rx);
        let joined = producer.join();
        #[cfg(test)]
        if let Ok((elapsed, blocks)) = &joined {
            metrics.report(*elapsed, wait, *blocks);
        }
        joined.map_err(|_| Error::Export("export geometry worker panicked".into()))?;
        result
    })
}

struct Message {
    origin: [u32; 2],
    extent: [u32; 2],
    plan: Result<ReconstructionPlan, Error>,
    #[cfg(test)]
    _charge: diagnostics::Charge,
}

#[cfg(test)]
mod diagnostics {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };
    #[derive(Clone, Default)]
    pub(super) struct Metrics {
        live: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
        plans: Arc<AtomicUsize>,
        peak_plans: Arc<AtomicUsize>,
    }
    impl Metrics {
        pub(super) fn charge(&self, bytes: usize) -> Charge {
            let live = self.live.fetch_add(bytes, Ordering::Relaxed) + bytes;
            self.peak.fetch_max(live, Ordering::Relaxed);
            let plans = self.plans.fetch_add(1, Ordering::Relaxed) + 1;
            self.peak_plans.fetch_max(plans, Ordering::Relaxed);
            Charge {
                bytes,
                live: self.live.clone(),
                plans: self.plans.clone(),
            }
        }
        pub(super) fn report(&self, planning: Duration, wait: Duration, blocks: usize) {
            assert_eq!(self.live.load(Ordering::Relaxed), 0);
            let peak = self.peak.load(Ordering::Relaxed);
            assert!(peak <= 2 * super::ReconstructionPlan::MAX_PAYLOAD_BYTES);
            assert_eq!(self.plans.load(Ordering::Relaxed), 0);
            assert!(self.peak_plans.load(Ordering::Relaxed) <= 2);
            println!(
                "export_geometry planning_seconds={:.6} consumer_wait_seconds={:.6} blocks={blocks} peak_completed_payload_bytes={peak} completed_payload_bound_bytes={} thread_stack_virtual_bytes={} note=overlapping_times_are_not_additive",
                planning.as_secs_f64(),
                wait.as_secs_f64(),
                2 * super::ReconstructionPlan::MAX_PAYLOAD_BYTES,
                super::STACK_BYTES
            );
        }
    }
    pub(super) struct Charge {
        bytes: usize,
        live: Arc<AtomicUsize>,
        plans: Arc<AtomicUsize>,
    }
    impl Drop for Charge {
        fn drop(&mut self) {
            self.live.fetch_sub(self.bytes, Ordering::Relaxed);
            self.plans.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
#[path = "gpu_export_pipeline_tests.rs"]
mod tests;
