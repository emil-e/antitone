use std::sync::{Arc, Mutex};

/// Shared by a hosted queue's UI and background submitters. Surface configuration
/// must exclude new submissions while wgpu drains the queue. Encoding, uploads and
/// explicit completion waits remain outside this gate. Device callbacks must not
/// re-enter it: wgpu can invoke them from configure or submit.
#[derive(Clone, Default)]
pub struct SubmissionGate(Arc<Mutex<()>>);

impl SubmissionGate {
    pub fn submit(
        &self,
        queue: &wgpu::Queue,
        commands: impl IntoIterator<Item = wgpu::CommandBuffer>,
    ) -> wgpu::SubmissionIndex {
        self.exclusive(|| queue.submit(commands))
    }

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
            let gate = SubmissionGate::default();
            let worker_gate = gate.clone();
            let worker_queue = queue.clone();
            let encoder = device.create_command_encoder(&Default::default());
            let commands = [encoder.finish()];
            let (ready, started) = mpsc::channel();
            let (finished, submitted) = mpsc::channel();
            let worker = gate.configure(|| {
                let worker = std::thread::spawn(move || {
                    ready.send(()).unwrap();
                    let index = worker_gate.submit(&worker_queue, commands);
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
        });
    }
}
