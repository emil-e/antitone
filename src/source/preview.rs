use super::{FileKey, PreviewResult};

#[cfg(test)]
mod tests;
use crate::embedded_preview::Preview;
use std::sync::{Arc, Mutex, OnceLock, Weak};

#[derive(Default)]
struct Flight {
    result: OnceLock<PreviewResult>,
    changed: event_listener::Event,
}

struct Entry {
    file: FileKey,
    pending: Weak<Flight>,
    ready: Weak<Preview>,
}

/// Shared preview acquisition outlives individual waiters, not their optional cache retention.
#[derive(Clone, Default)]
pub(crate) struct PreviewLoader(Arc<Mutex<Vec<Entry>>>);

impl PreviewLoader {
    pub(crate) async fn load(
        &self,
        file: FileKey,
        read: impl FnOnce() -> PreviewResult + Send + 'static,
    ) -> PreviewResult {
        let (flight, start) = {
            let mut entries = self.0.lock().unwrap();
            entries.retain(|entry| {
                entry.pending.strong_count() != 0 || entry.ready.strong_count() != 0
            });
            if let Some(entry) = entries.iter().find(|entry| entry.file == file) {
                if let Some(preview) = entry.ready.upgrade() {
                    return Ok(Some(preview));
                }
                if let Some(flight) = entry.pending.upgrade() {
                    (flight, false)
                } else {
                    let flight = Arc::new(Flight::default());
                    entries.retain(|entry| entry.file != file);
                    entries.push(Entry {
                        file: file.clone(),
                        pending: Arc::downgrade(&flight),
                        ready: Weak::new(),
                    });
                    (flight, true)
                }
            } else {
                let flight = Arc::new(Flight::default());
                entries.push(Entry {
                    file: file.clone(),
                    pending: Arc::downgrade(&flight),
                    ready: Weak::new(),
                });
                (flight, true)
            }
        };
        if start {
            let flight = flight.clone();
            let entries = self.0.clone();
            let published_file = file.clone();
            // Submission owns decode inputs even if the first waiter departs before dispatch.
            blocking::unblock(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let result = read();
                    if let Ok(Some(preview)) = &result
                        && let Some(entry) = entries
                            .lock()
                            .unwrap()
                            .iter_mut()
                            .find(|entry| entry.file == published_file)
                    {
                        entry.ready = Arc::downgrade(preview);
                    }
                    result
                }))
                .unwrap_or_else(|_| Err("Preview decoding worker panicked".into()));
                let _ = flight.result.set(result);
                flight.changed.notify(usize::MAX);
            })
            .detach();
        }
        loop {
            let changed = flight.changed.listen();
            if let Some(result) = flight.result.get() {
                return result.clone();
            }
            changed.await;
        }
    }
}
