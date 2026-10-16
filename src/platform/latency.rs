use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

// All timestamps share this host's monotonic origin. Completion is callback observation,
// not a GPU timestamp or scanout; the diagnostic polls without requesting redraws.
pub(super) struct Trace {
    origin: Instant,
    next_frame: u64,
    input: Option<Input>,
    send: mpsc::Sender<Completion>,
    receive: mpsc::Receiver<Completion>,
    outstanding: Arc<AtomicU64>,
    peak_outstanding: u64,
    peak_resource_bytes: usize,
}

#[derive(Clone, Copy)]
struct Input {
    first: Instant,
    last: Instant,
    count: u64,
}

pub(super) struct Frame {
    id: u64,
    input: Option<Input>,
    pub draw: Instant,
    pub acquire_status: &'static str,
    pub acquire_start: Option<Instant>,
    pub acquire_end: Option<Instant>,
    pub ui_start: Option<Instant>,
    pub ui_end: Option<Instant>,
    pub encode_start: Option<Instant>,
    pub submit_start: Option<Instant>,
    pub submit_end: Option<Instant>,
    pub present_end: Option<Instant>,
}

struct Completion {
    frame: u64,
    at: Instant,
    remaining: u64,
}

impl Trace {
    pub(super) fn from_env() -> Option<Self> {
        (std::env::var_os("ANTITONE_LATENCY_TRACE").as_deref() == Some(std::ffi::OsStr::new("1")))
            .then(Self::new)
    }

    fn new() -> Self {
        let (send, receive) = mpsc::channel();
        println!(
            "latency_trace clock=host_monotonic_ns input_origin=winit_delivery completion=callback_observation_not_gpu_timestamp presentation=present_call_not_scanout poll_interval_ms=2 metal_bytes=allocated_resources_not_process_memory"
        );
        Self {
            origin: Instant::now(),
            next_frame: 0,
            input: None,
            send,
            receive,
            outstanding: Arc::new(AtomicU64::new(0)),
            peak_outstanding: 0,
            peak_resource_bytes: 0,
        }
    }

    pub(super) fn input(&mut self) {
        let at = Instant::now();
        match &mut self.input {
            Some(input) => {
                input.last = at;
                input.count += 1;
            }
            None => {
                self.input = Some(Input {
                    first: at,
                    last: at,
                    count: 1,
                })
            }
        }
    }

    pub(super) fn begin(&mut self) -> Frame {
        self.next_frame += 1;
        Frame {
            id: self.next_frame,
            input: self.input.take(),
            draw: Instant::now(),
            acquire_status: "zero_size",
            acquire_start: None,
            acquire_end: None,
            ui_start: None,
            ui_end: None,
            encode_start: None,
            submit_start: None,
            submit_end: None,
            present_end: None,
        }
    }

    pub(super) fn submitting(&mut self, frame: &mut Frame) {
        frame.submit_start = Some(Instant::now());
        self.peak_outstanding = self
            .peak_outstanding
            .max(self.outstanding.fetch_add(1, Ordering::Relaxed) + 1);
    }

    pub(super) fn completion(&self, frame: &Frame) -> impl FnOnce() + Send + 'static + use<> {
        let id = frame.id;
        let outstanding = self.outstanding.clone();
        let send = self.send.clone();
        move || {
            let at = Instant::now();
            let remaining = outstanding.fetch_sub(1, Ordering::Relaxed) - 1;
            let _ = send.send(Completion {
                frame: id,
                at,
                remaining,
            });
        }
    }

    pub(super) fn finish(
        &mut self,
        frame: Frame,
        revision: Option<(u64, u64)>,
        device: &wgpu::Device,
    ) {
        let bytes = crate::gpu::allocated_resource_bytes(device);
        self.peak_resource_bytes = self.peak_resource_bytes.max(bytes.unwrap_or(0));
        let ns =
            |at: Option<Instant>| at.map_or_else(|| "-".to_owned(), |at| self.ns(at).to_string());
        println!(
            "latency_frame id={} input_count={} input_first_ns={} input_last_ns={} draw_ns={} acquire_status={} acquire_start_ns={} acquire_end_ns={} ui_start_ns={} ui_end_ns={} encode_start_ns={} submit_start_ns={} submit_end_ns={} present_end_ns={} request={} revision={} metal_resource_bytes={}",
            frame.id,
            frame.input.map_or(0, |input| input.count),
            ns(frame.input.map(|input| input.first)),
            ns(frame.input.map(|input| input.last)),
            self.ns(frame.draw),
            frame.acquire_status,
            ns(frame.acquire_start),
            ns(frame.acquire_end),
            ns(frame.ui_start),
            ns(frame.ui_end),
            ns(frame.encode_start),
            ns(frame.submit_start),
            ns(frame.submit_end),
            ns(frame.present_end),
            revision.map_or_else(|| "-".to_owned(), |r| r.0.to_string()),
            revision.map_or_else(|| "-".to_owned(), |r| r.1.to_string()),
            bytes.map_or_else(|| "-".to_owned(), |bytes| bytes.to_string())
        );
    }

    pub(super) fn configured(&self, start: Instant, width: u32, height: u32) {
        println!(
            "latency_configure start_ns={} end_ns={} width={width} height={height}",
            self.ns(start),
            self.ns(Instant::now())
        );
    }

    pub(super) fn flush(&self) {
        for done in self.receive.try_iter() {
            println!(
                "latency_completion id={} observed_ns={} remaining={}",
                done.frame,
                self.ns(done.at),
                done.remaining
            );
        }
    }

    pub(super) fn pending(&self) -> bool {
        self.outstanding.load(Ordering::Relaxed) != 0
    }

    pub(super) fn poll_deadline(&self) -> Option<Instant> {
        self.pending()
            .then(|| Instant::now() + Duration::from_millis(2))
    }

    pub(super) fn summary(&self) {
        self.flush();
        println!(
            "latency_summary frames={} peak_uncompleted_frames={} peak_sampled_metal_resource_bytes={} remaining={}",
            self.next_frame,
            self.peak_outstanding,
            self.peak_resource_bytes,
            self.outstanding.load(Ordering::Relaxed)
        );
    }

    fn ns(&self, at: Instant) -> u128 {
        at.duration_since(self.origin).as_nanos()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivered_input_is_consumed_once_and_frame_ids_are_distinct() {
        let mut trace = Trace::new();
        trace.input();
        trace.input();
        let first = trace.begin();
        assert_eq!(first.input.unwrap().count, 2);
        assert!(first.input.unwrap().first <= first.input.unwrap().last);
        let second = trace.begin();
        assert!(second.input.is_none());
        assert_eq!(second.id, first.id + 1);
        assert!(second.draw >= first.draw);
    }

    #[test]
    fn real_queue_completion_clears_pending_without_another_draw() {
        pollster::block_on(async {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::METAL,
                ..wgpu::InstanceDescriptor::new_without_display_handle()
            });
            let adapter = instance.request_adapter(&Default::default()).await.unwrap();
            let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
            let mut trace = Trace::new();
            let mut frame = trace.begin();
            trace.submitting(&mut frame);
            let encoder = device.create_command_encoder(&Default::default());
            let index = crate::gpu::SubmissionGate::default().submit_with_completion(
                &queue,
                [encoder.finish()],
                trace.completion(&frame),
            );
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(index),
                    timeout: Some(Duration::from_secs(5)),
                })
                .unwrap();
            let done = trace.receive.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(done.frame, frame.id);
            assert!(done.at >= frame.submit_start.unwrap());
            assert_eq!(done.remaining, 0);
            assert!(!trace.pending());
            assert!(trace.poll_deadline().is_none());
            assert_eq!(trace.next_frame, 1);
        });
    }

    #[test]
    fn completion_polling_has_a_deadline_only_while_work_is_outstanding() {
        let mut trace = Trace::new();
        assert!(trace.poll_deadline().is_none());
        let mut first = trace.begin();
        trace.submitting(&mut first);
        let mut second = trace.begin();
        trace.submitting(&mut second);
        assert_eq!(trace.peak_outstanding, 2);
        assert!(trace.poll_deadline().is_some());
        trace.outstanding.fetch_sub(2, Ordering::Relaxed);
        assert!(trace.poll_deadline().is_none());
        assert_eq!(trace.peak_outstanding, 2);
    }
}
