//! Opt-in pass counters without additional per-tile submissions or completion waits.
use super::*;
use objc2_metal::MTLDevice;
use std::{
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

#[test]
fn calibration_uses_sample_call_brackets_and_rejects_incompatible_cpu_intervals() {
    let base = Instant::now();
    for (first, second) in [([0, 0], [20, 20]), ([0, 4], [24, 24]), ([0, 0], [20, 24])] {
        let before = first.map(|ms| base + Duration::from_millis(ms));
        let after = second.map(|ms| base + Duration::from_millis(ms));
        assert!(validate_cpu_calibration(20_000_000, before, after).is_ok());
        for incompatible in [0, 10_000_000, 30_000_000] {
            assert!(validate_cpu_calibration(incompatible, before, after).is_err());
        }
    }
    for outside_tolerance in [18_700_000, 21_300_000] {
        assert!(
            validate_cpu_calibration(
                outside_tolerance,
                [base; 2],
                [base + Duration::from_millis(20); 2]
            )
            .is_err()
        );
    }
    assert!(
        validate_cpu_calibration(
            20_000_000,
            [base + Duration::from_millis(1), base],
            [base + Duration::from_millis(20); 2],
        )
        .is_err()
    );
}

#[test]
fn counter_intervals_reject_zero_reversed_stale_future_and_unbracketed_samples() {
    let clock = Timebase {
        nanoseconds: 1.,
        nanoseconds_error: 0.,
        anchor: 100,
        fault: AtomicBool::new(false),
    };
    assert_eq!(
        clock
            .interval([200, 500], Some([100, 150]), 600, 1e-6)
            .unwrap(),
        300e-9
    );
    for (words, previous, native) in [
        ([0, 500], None, 600),
        ([200, 0], None, 600),
        ([200, 199], None, 600),
        ([99, 500], None, 600),
        ([200, 700], None, 600),
        ([200, 500], Some([200, 500]), 600),
    ] {
        assert!(clock.interval(words, previous, native, 0.01).is_err());
    }
    assert!(
        clock
            .interval([200, 2_000_000_000], None, 3_000_000_000, 0.01)
            .is_err()
    );
    assert!(clock.interval([200, 500], None, 600, f64::NAN).is_err());
    assert!(
        clock
            .interval([200, 500], None, 600, f64::INFINITY)
            .is_err()
    );
    assert!(clock.interval([200, 500], None, 600, -1.).is_err());
}

#[test]
fn placement_rejects_advancing_intervals_outside_their_owning_submission() {
    let clock = Timebase {
        nanoseconds: 1.,
        nanoseconds_error: 0.,
        anchor: 100,
        fault: AtomicBool::new(false),
    };
    let base = Instant::now();
    let host = HostBracket {
        submitted: base + Duration::from_millis(10),
        completed: base + Duration::from_millis(20),
    };
    let reference = ClockReference {
        gpu: 100_000_000,
        before: base + Duration::from_millis(100),
        after: base + Duration::from_millis(100) + Duration::from_micros(1),
    };
    assert!(
        clock
            .placement([12_000_000, 18_000_000], &host, &reference)
            .is_ok()
    );
    for words in [
        [1_000_000, 5_000_000],
        [30_000_000, 35_000_000],
        [9_000_000, 13_000_000],
        [17_000_000, 21_000_000],
    ] {
        assert!(
            clock
                .interval(words, Some([100, 200]), 110_000_000, 0.01)
                .is_ok(),
            "duration plausibility alone must not accept shifted ownership"
        );
        assert!(clock.placement(words, &host, &reference).is_err());
    }
}

fn validate_cpu_calibration(
    nanoseconds: u64,
    before: [Instant; 2],
    after: [Instant; 2],
) -> Result<(), Error> {
    if nanoseconds == 0 || before[0] > before[1] || before[1] > after[0] || after[0] > after[1] {
        return Err(Error::Gpu("invalid CPU calibration brackets".into()));
    }
    // Each native sample falls inside its call, not necessarily at the host call boundary.
    // Scheduling or device-lock waits outside the samples must not become clock mismatch.
    let minimum = after[0].duration_since(before[1]).as_secs_f64();
    let maximum = after[1].duration_since(before[0]).as_secs_f64();
    let cpu_seconds = nanoseconds as f64 / 1e9;
    let tolerance = maximum * 0.01 + 0.001;
    if cpu_seconds < minimum - tolerance || cpu_seconds > maximum + tolerance {
        return Err(Error::Gpu(
            "CPU calibration does not match host sample brackets".into(),
        ));
    }
    Ok(())
}

fn native_pair(device: &wgpu::Device) -> Result<[u64; 2], Error> {
    // Device inspection does not enter wgpu's forbidden raw command-encoding mode.
    let device = unsafe { device.as_hal::<wgpu::hal::api::Metal>() }
        .ok_or_else(|| Error::Gpu("Metal clock calibration unavailable".into()))?;
    let mut cpu = 0;
    let mut gpu = 0;
    unsafe {
        device
            .raw_device()
            .sampleTimestamps_gpuTimestamp(NonNull::from(&mut cpu), NonNull::from(&mut gpu));
    }
    if cpu == 0 || gpu == 0 {
        return Err(Error::Gpu(
            "Metal returned an unavailable clock pair".into(),
        ));
    }
    Ok([cpu, gpu])
}

struct HostBracket {
    submitted: Instant,
    completed: Instant,
}

struct ClockReference {
    gpu: u64,
    before: Instant,
    after: Instant,
}

pub(crate) struct Timebase {
    nanoseconds: f64,
    nanoseconds_error: f64,
    anchor: u64,
    fault: AtomicBool,
}

impl Timebase {
    #[cfg(feature = "editor")]
    pub fn invalid_for_collection_test(&self) -> Arc<Self> {
        Arc::new(Self {
            nanoseconds: f64::NAN,
            nanoseconds_error: 0.,
            anchor: self.anchor,
            fault: AtomicBool::new(false),
        })
    }

    pub fn require_valid(&self) -> Result<(), Error> {
        if self.fault.load(Ordering::Relaxed) {
            Err(Error::Gpu(
                "GPU counter collection failed; no complete timing is available".into(),
            ))
        } else {
            Ok(())
        }
    }

    fn placement(
        &self,
        words: [u64; 2],
        host: &HostBracket,
        reference: &ClockReference,
    ) -> Result<(), Error> {
        let convert = |value, scale| -> Result<Duration, Error> {
            let ticks = reference
                .gpu
                .checked_sub(value)
                .ok_or_else(|| Error::Gpu("GPU interval is after native clock sample".into()))?;
            Duration::try_from_secs_f64(ticks as f64 * scale / 1e9)
                .map_err(|error| Error::Gpu(error.to_string()))
        };
        // Bound the clock-call placement and observed calibration spread; no shared epoch
        // between std::Instant and Metal's paired CPU timestamps is assumed.
        let start_age = convert(words[0], self.nanoseconds)?;
        let end_age = convert(words[1], self.nanoseconds)?;
        let start_error = convert(words[0], self.nanoseconds_error)? + Duration::from_nanos(1);
        let end_error = convert(words[1], self.nanoseconds_error)? + Duration::from_nanos(1);
        let latest_start = reference
            .after
            .checked_sub(start_age)
            .and_then(|time| time.checked_add(start_error))
            .ok_or_else(|| Error::Gpu("GPU start placement overflow".into()))?;
        let earliest_end = reference
            .before
            .checked_sub(end_age)
            .and_then(|time| time.checked_sub(end_error))
            .ok_or_else(|| Error::Gpu("GPU end placement overflow".into()))?;
        if latest_start < host.submitted || earliest_end > host.completed {
            return Err(Error::Gpu(format!(
                "GPU pass is outside its owning host submission/completion bracket: raw={words:?}"
            )));
        }
        Ok(())
    }

    fn interval(
        &self,
        words: [u64; 2],
        previous: Option<[u64; 2]>,
        native_after: u64,
        host: f64,
    ) -> Result<f64, Error> {
        if words.contains(&0)
            || words[0] < self.anchor
            || words[1] <= words[0]
            || words[1] > native_after
            || previous.is_some_and(|old| words[0] <= old[1])
        {
            return Err(Error::Gpu(format!(
                "invalid/stale GPU pass counters: {words:?} previous={previous:?} anchor={} native_after={native_after}",
                self.anchor
            )));
        }
        let seconds = (words[1] - words[0]) as f64 * self.nanoseconds / 1e9;
        if !seconds.is_finite()
            || seconds <= 0.
            || !host.is_finite()
            || host < 0.
            || seconds > host * 1.01 + 0.001
        {
            return Err(Error::Gpu(format!(
                "GPU pass interval does not fit submission/completion bracket: seconds={seconds} host={host}"
            )));
        }
        Ok(seconds)
    }

    #[cfg(feature = "editor")]
    pub fn calibrate(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Arc<Self>, Error> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return Err(Error::Gpu("timestamp feature was not requested".into()));
        }
        let mut scales = Vec::new();
        let mut anchor = 0;
        for _ in 0..3 {
            let before_start = Instant::now();
            let before = native_pair(device)?;
            let before_end = Instant::now();
            std::thread::sleep(Duration::from_millis(20));
            let after_start = Instant::now();
            let after = native_pair(device)?;
            let after_end = Instant::now();
            let elapsed = after_end.duration_since(before_start).as_secs_f64();
            println!(
                "export_gpu_clock_raw before={before:?} after={after:?} host_seconds={elapsed:.9} queue_reported_period={:.9}",
                queue.get_timestamp_period()
            );
            let cpu_nanoseconds = after[0]
                .checked_sub(before[0])
                .filter(|v| *v != 0)
                .ok_or_else(|| Error::Gpu("nonmonotonic CPU calibration".into()))?;
            let gpu_ticks = after[1]
                .checked_sub(before[1])
                .filter(|v| *v != 0)
                .ok_or_else(|| Error::Gpu("nonmonotonic GPU calibration".into()))?;
            // Metal's paired CPU timestamp is nanoseconds, not raw Mach absolute-time ticks.
            let cpu_seconds = cpu_nanoseconds as f64 / 1e9;
            validate_cpu_calibration(
                cpu_nanoseconds,
                [before_start, before_end],
                [after_start, after_end],
            )?;
            let scale = cpu_nanoseconds as f64 / gpu_ticks as f64;
            if !scale.is_finite() || scale <= 0. {
                return Err(Error::Gpu("invalid GPU clock scale".into()));
            }
            println!(
                "export_gpu_clock cpu_before={} gpu_before={} cpu_after={} gpu_after={} host_seconds={elapsed:.9} cpu_seconds={cpu_seconds:.9} gpu_nanoseconds_per_tick={scale:.9} queue_reported_period={:.9}",
                before[0],
                before[1],
                after[0],
                after[1],
                queue.get_timestamp_period()
            );
            scales.push(scale);
            anchor = after[1];
        }
        scales.sort_by(f64::total_cmp);
        if scales[2] / scales[0] > 1.01 {
            return Err(Error::Gpu(
                "GPU timestamp calibration varies by more than one percent".into(),
            ));
        }
        Ok(Arc::new(Self {
            nanoseconds: scales[1],
            nanoseconds_error: (scales[0] - scales[1])
                .abs()
                .max((scales[2] - scales[1]).abs()),
            anchor,
            fault: AtomicBool::new(false),
        }))
    }
}

#[derive(Default, Clone)]
pub(crate) struct TimestampStats {
    processing: f64,
    calls: u64,
    collection_submissions: u64,
}
impl TimestampStats {
    pub fn report(&self) {
        println!(
            "export_gpu_timestamp phase=Processing gpu_seconds={:.9} calls={} extra_collection_submissions={} note=calibrated_pass_interval_not_exclusive_alu",
            self.processing, self.calls, self.collection_submissions
        );
        println!(
            "export_gpu_timestamp phase=Rendering gpu_seconds=unavailable note=fragment_end_counter_zero_in_retained_authored_probe_not_inferred_from_other_stages"
        );
    }
}

struct CompletedQuery {
    index: u32,
    host: HostBracket,
}

pub(crate) struct TimestampProbe {
    queries: [wgpu::QuerySet; 2],
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    timebase: Arc<Timebase>,
    submitted: Option<Instant>,
    next_index: u32,
    record: Option<CompletedQuery>,
    pending: Option<mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>>,
    previous: Option<[u64; 2]>,
    pub stats: TimestampStats,
}

impl TimestampProbe {
    pub fn new(device: &wgpu::Device, timebase: Arc<Timebase>) -> Self {
        Self {
            queries: std::array::from_fn(|_| {
                device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("original processing counters"),
                    ty: wgpu::QueryType::Timestamp,
                    count: wgpu::QUERY_SET_MAX_QUERIES,
                })
            }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 256,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            readback: device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 16,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
            timebase,
            submitted: None,
            next_index: 0,
            record: None,
            pending: None,
            previous: None,
            stats: TimestampStats::default(),
        }
    }

    pub fn processing_queries(&self) -> Result<(&wgpu::QuerySet, u32), Error> {
        let index = self.next_index;
        let set = self
            .queries
            .get((index / wgpu::QUERY_SET_MAX_QUERIES) as usize)
            .ok_or_else(|| Error::Gpu("timestamp query capacity exceeded".into()))?;
        Ok((set, index % wgpu::QUERY_SET_MAX_QUERIES))
    }

    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        let Some(record) = &self.record else {
            return;
        };
        let queries = &self.queries[(record.index / wgpu::QUERY_SET_MAX_QUERIES) as usize];
        let index = record.index % wgpu::QUERY_SET_MAX_QUERIES;
        // Resolve only counters whose submission has already completed. Same-command-buffer
        // resolution returned stale/zero data on this device, even with unique sample slots.
        encoder.resolve_query_set(queries, index..index + 2, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.readback, 0, 16);
    }

    pub fn submitting(&mut self) {
        assert!(self.submitted.is_none());
        self.submitted = Some(Instant::now());
    }

    pub fn arm(&mut self) {
        assert!(self.pending.is_none());
        if self.record.is_none() {
            return;
        }
        let (tx, rx) = mpsc::sync_channel(1);
        self.readback
            .map_async(wgpu::MapMode::Read, .., move |result| {
                let _ = tx.send(result);
            });
        self.pending = Some(rx);
    }

    pub fn complete(&mut self, device: &wgpu::Device) -> Result<(), Error> {
        let current = CompletedQuery {
            index: self.next_index,
            host: HostBracket {
                submitted: self.submitted.take().expect("missing counter submission"),
                completed: Instant::now(),
            },
        };
        if self.record.is_some() {
            self.consume(device)?;
        }
        self.record = Some(current);
        self.next_index += 2;
        Ok(())
    }

    pub fn flush(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gate: &SubmissionGate,
    ) -> Result<(), Error> {
        if self.record.is_none() {
            return Ok(());
        }
        let result = (|| {
            let mut encoder = device.create_command_encoder(&Default::default());
            self.resolve(&mut encoder);
            let submission = gate.submit(queue, [encoder.finish()]);
            self.stats.collection_submissions += 1;
            self.arm();
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submission),
                    timeout: Some(Duration::from_secs(2)),
                })
                .map_err(|error| Error::Gpu(error.to_string()))?;
            self.consume(device)?;
            assert_eq!(self.stats.calls, u64::from(self.next_index / 2));
            Ok(())
        })();
        if result.is_err() {
            self.timebase.fault.store(true, Ordering::Relaxed);
            // No mapped view exists before consume; dropping this probe owns cancellation
            // if a device/poll failure left its mapping pending.
            drop(self.pending.take());
        }
        result
    }

    fn consume(&mut self, device: &wgpu::Device) -> Result<(), Error> {
        let record = self.record.take().expect("no completed counter submission");
        let mapped = self
            .pending
            .take()
            .expect("missing counter mapping")
            .recv_timeout(Duration::from_secs(2))
            .map_err(|error| Error::Gpu(error.to_string()))
            .and_then(|result| result.map_err(|error| Error::Gpu(error.to_string())));
        if let Err(error) = mapped {
            self.timebase.fault.store(true, Ordering::Relaxed);
            return Err(error);
        }
        let result = (|| {
            let mapped = self
                .readback
                .get_mapped_range(..)
                .map_err(|e| Error::Gpu(e.to_string()))?;
            let words: [u64; 2] = bytemuck::cast_slice::<u8, u64>(&mapped).try_into().unwrap();
            drop(mapped);
            let before = Instant::now();
            let native = native_pair(device)?;
            let after = Instant::now();
            let host = record
                .host
                .completed
                .duration_since(record.host.submitted)
                .as_secs_f64();
            let processing = self
                .timebase
                .interval(words, self.previous, native[1], host)?;
            self.timebase.placement(
                words,
                &record.host,
                &ClockReference {
                    gpu: native[1],
                    before,
                    after,
                },
            )?;
            if self.stats.calls == 0 {
                println!(
                    "export_gpu_timestamp first_raw={words:?} native_after={native:?} host_bracket_seconds={host:.9} calibrated_nanoseconds_per_tick={:.9} native_before_from_submit_seconds={:.9} native_after_from_submit_seconds={:.9}",
                    self.timebase.nanoseconds,
                    before.duration_since(record.host.submitted).as_secs_f64(),
                    after.duration_since(record.host.submitted).as_secs_f64()
                );
            }
            self.previous = Some(words);
            self.stats.processing += processing;
            self.stats.calls += 1;
            Ok(())
        })();
        self.readback.unmap();
        if result.is_err() {
            self.timebase.fault.store(true, Ordering::Relaxed);
        }
        result
    }
}
