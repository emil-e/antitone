use std::sync::{Arc, Mutex};
#[cfg(test)]
type SubmissionObserver = Arc<dyn Fn() + Send + Sync>;

/// Shared by a hosted queue's UI and background submitters. Surface configuration
/// must exclude new submissions while wgpu drains the queue. Encoding, uploads and
/// explicit completion waits remain outside this gate. Device callbacks must not
/// re-enter it: wgpu can invoke them from configure or submit.
#[derive(Clone, Default)]
pub struct SubmissionGate(
    Arc<Mutex<()>>,
    #[cfg(any(feature = "ui", test))] Arc<std::sync::atomic::AtomicUsize>,
    #[cfg(test)] Arc<Mutex<Option<SubmissionObserver>>>,
);

impl SubmissionGate {
    pub fn submit(
        &self,
        queue: &wgpu::Queue,
        commands: impl IntoIterator<Item = wgpu::CommandBuffer>,
    ) -> wgpu::SubmissionIndex {
        let index = self.exclusive(|| queue.submit(commands));
        #[cfg(test)]
        self.observe_submission();
        index
    }

    /// Attach completion before a background submitter can advance the queue fence.
    #[cfg(any(feature = "ui", test))]
    pub(crate) fn submit_with_completion(
        &self,
        queue: &wgpu::Queue,
        commands: impl IntoIterator<Item = wgpu::CommandBuffer>,
        completion: impl FnOnce() + Send + 'static,
    ) -> wgpu::SubmissionIndex {
        let index = self.exclusive(|| {
            use std::sync::atomic::Ordering;
            self.1.fetch_add(1, Ordering::Relaxed);
            let pending = self.1.clone();
            let index = queue.submit(commands);
            queue.on_submitted_work_done(move || {
                completion();
                pending.fetch_sub(1, Ordering::Release);
            });
            index
        });
        #[cfg(test)]
        self.observe_submission();
        index
    }

    #[cfg(test)]
    pub(crate) fn observe_submissions(&self, observer: SubmissionObserver) {
        *self.2.lock().unwrap() = Some(observer);
    }

    #[cfg(test)]
    fn observe_submission(&self) {
        let observer = self.2.lock().unwrap().clone();
        if let Some(observer) = observer {
            observer();
        }
    }

    #[cfg(any(feature = "ui", test))]
    pub(crate) fn pending(&self) -> bool {
        self.1.load(std::sync::atomic::Ordering::Acquire) != 0
    }

    #[cfg(any(feature = "ui", test))]
    pub(crate) fn configure<R>(&self, configure: impl FnOnce() -> R) -> R {
        self.exclusive(configure)
    }

    fn exclusive<R>(&self, operation: impl FnOnce() -> R) -> R {
        let _guard = self.0.lock().unwrap();
        operation()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, time::Duration};

    #[test]
    fn clones_share_the_gate_and_release_it_after_configuration() {
        let gate = SubmissionGate::default();
        let worker = gate.clone();
        assert_eq!(
            gate.exclusive(|| {
                assert!(worker.0.try_lock().is_err());
                42
            }),
            42
        );
        assert!(worker.0.try_lock().is_ok());
    }

    #[test]
    fn queue_submission_waits_until_configuration_releases_the_shared_gate() {
        pollster::block_on(async {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::METAL,
                ..wgpu::InstanceDescriptor::new_without_display_handle()
            });
            let adapter = instance.request_adapter(&Default::default()).await.unwrap();
            let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
            for observed in [false, true] {
                let gate = SubmissionGate::default();
                let worker_gate = gate.clone();
                let worker_queue = queue.clone();
                let encoder = device.create_command_encoder(&Default::default());
                let commands = [encoder.finish()];
                let (ready, started) = mpsc::channel();
                let (finished, submitted) = mpsc::channel();
                let (completed, completion) = mpsc::channel();
                let worker = gate.configure(|| {
                    let worker = std::thread::spawn(move || {
                        ready.send(()).unwrap();
                        let index = if observed {
                            worker_gate.submit_with_completion(&worker_queue, commands, move || {
                                completed.send(()).unwrap();
                            })
                        } else {
                            worker_gate.submit(&worker_queue, commands)
                        };
                        finished.send(index).unwrap();
                    });
                    started.recv_timeout(Duration::from_secs(5)).unwrap();
                    assert!(matches!(
                        submitted.recv_timeout(Duration::from_millis(100)),
                        Err(mpsc::RecvTimeoutError::Timeout)
                    ));
                    worker
                });
                let index = submitted.recv_timeout(Duration::from_secs(5)).unwrap();
                worker.join().unwrap();
                device
                    .poll(wgpu::PollType::Wait {
                        submission_index: Some(index),
                        timeout: None,
                    })
                    .unwrap();
                if observed {
                    completion.recv_timeout(Duration::from_secs(5)).unwrap();
                }
                assert!(!gate.pending());
            }
        });
    }
}
