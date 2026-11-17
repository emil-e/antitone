//! Submitted GPU work owns its resources until completion; retired storage shares working admission.
use super::*;
#[cfg(feature = "export")]
use crate::export_timing::timed;
#[cfg(not(feature = "export"))]
macro_rules! timed {
    ($stage:ident,$body:expr) => {
        $body
    };
}
use std::sync::{
    Weak,
    atomic::{AtomicBool, Ordering},
};

// Bound driver/descriptor ownership even when many submissions reuse the same allocations.
const QUEUED_SUBMISSIONS: usize = 32;

#[derive(Default)]
pub(super) struct GpuRun {
    active: Mutex<Vec<Weak<Allocations>>>,
    state: Mutex<State>,
}
#[derive(Default)]
struct State {
    device: Option<wgpu::Device>,
    queued: VecDeque<Submission>,
    #[cfg(test)]
    submitted: usize,
    #[cfg(test)]
    waited: usize,
    #[cfg(test)]
    peak_queued: usize,
}
struct Submission {
    index: wgpu::SubmissionIndex,
    pending: Arc<Pending>,
}
struct Pending {
    complete: AtomicBool,
    resources: Allocations,
}

impl GpuRun {
    pub(super) fn activate(&self, resources: Allocations) -> Arc<Allocations> {
        let resources = Arc::new(resources);
        let mut active = self.active.lock().unwrap();
        active.retain(|value| value.strong_count() != 0);
        active.push(Arc::downgrade(&resources));
        resources
    }

    pub(super) fn admit(
        &self,
        device: &wgpu::Device,
        required: u64,
        allowance: u64,
    ) -> Result<(), Error> {
        if required > allowance {
            return Err(Error::WorkingAllowance(
                "GPU working allocation exceeds its allowance",
            ));
        }
        loop {
            if self.state.lock().unwrap().queued.is_empty() {
                return Ok(());
            }
            device
                .poll(wgpu::PollType::Poll)
                .map_err(|error| Error::Gpu(error.to_string()))?;
            let mut state = self.state.lock().unwrap();
            state
                .queued
                .retain(|value| !value.pending.complete.load(Ordering::Acquire));
            let mut live = Allocations::default();
            self.active.lock().unwrap().retain(|value| {
                if let Some(value) = value.upgrade() {
                    live.extend(&value);
                    true
                } else {
                    false
                }
            });
            let mut pending = Allocations::default();
            for value in &state.queued {
                pending.extend(&value.pending.resources);
            }
            let extra = pending.bytes_except(&live);
            if extra
                .checked_add(required)
                .is_some_and(|bytes| bytes <= allowance)
                && state.queued.len() < QUEUED_SUBMISSIONS
            {
                return Ok(());
            }
            let index = state
                .queued
                .front()
                .expect("pending working storage")
                .index
                .clone();
            #[cfg(test)]
            {
                state.waited += 1;
            }
            drop(state);
            timed!(
                DeviceWait,
                device.poll(wgpu::PollType::Wait {
                    submission_index: Some(index),
                    timeout: Some(std::time::Duration::from_secs(10)),
                })
            )
            .map_err(|error| Error::Gpu(error.to_string()))?;
        }
    }

    pub(super) fn submit(
        &self,
        device: &wgpu::Device,
        (queue, gate): (&wgpu::Queue, &crate::gpu::submission::SubmissionGate),
        encoder: wgpu::CommandEncoder,
        resources: Allocations,
        (required, allowance): (u64, u64),
        submitted: impl FnOnce(),
    ) -> Result<(), Error> {
        let _active = self.activate(resources.clone());
        self.admit(device, required, allowance)?;
        let pending = Arc::new(Pending {
            complete: AtomicBool::new(false),
            resources,
        });
        let completion = pending.clone();
        let index = timed!(
            Submit,
            gate.submit_with_completion(queue, [encoder.finish()], move || {
                completion.complete.store(true, Ordering::Release);
            })
        );
        let mut state = self.state.lock().unwrap();
        state.device = Some(device.clone());
        state.queued.push_back(Submission { index, pending });
        #[cfg(test)]
        {
            state.submitted += 1;
            state.peak_queued = state.peak_queued.max(state.queued.len());
        }
        drop(state);
        // Establish completion ownership before publication, including publication unwinding.
        submitted();
        Ok(())
    }

    pub(super) fn finish(&self) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();
        state
            .queued
            .retain(|value| !value.pending.complete.load(Ordering::Acquire));
        let Some(last) = state.queued.back() else {
            return Ok(());
        };
        let index = last.index.clone();
        let device = state.device.as_ref().unwrap().clone();
        #[cfg(test)]
        {
            state.waited += 1;
        }
        drop(state);
        timed!(
            DeviceWait,
            device.poll(wgpu::PollType::Wait {
                submission_index: Some(index),
                timeout: Some(std::time::Duration::from_secs(10)),
            })
        )
        .map_err(|error| Error::Gpu(error.to_string()))?;
        self.state
            .lock()
            .unwrap()
            .queued
            .retain(|value| !value.pending.complete.load(Ordering::Acquire));
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn peak_queued(&self) -> usize {
        self.state.lock().unwrap().peak_queued
    }

    #[cfg(test)]
    pub(super) fn owns(&self, image: &Arc<ImageValue>) -> bool {
        self.state
            .lock()
            .unwrap()
            .queued
            .iter()
            .any(|value| value.pending.resources.owns(image))
    }

    #[cfg(test)]
    pub(super) fn retired_bytes(&self) -> u64 {
        let mut live = Allocations::default();
        for value in self.active.lock().unwrap().iter().filter_map(Weak::upgrade) {
            live.extend(&value);
        }
        let mut pending = Allocations::default();
        for value in &self.state.lock().unwrap().queued {
            pending.extend(&value.pending.resources);
        }
        pending.bytes_except(&live)
    }

    #[cfg(test)]
    pub(super) fn counts(&self) -> (usize, usize) {
        let state = self.state.lock().unwrap();
        (state.submitted, state.waited)
    }
}

impl Drop for GpuRun {
    fn drop(&mut self) {
        let state = self
            .state
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        if let (Some(device), Some(last)) = (&state.device, state.queued.back()) {
            // Only worker execution enqueues here. Cancellation withdraws publication rights,
            // not physical ownership or its admission; cleanup drains before the worker retires.
            let _ = timed!(
                DeviceWait,
                device.poll(wgpu::PollType::Wait {
                    submission_index: Some(last.index.clone()),
                    timeout: None,
                })
            );
        }
    }
}
