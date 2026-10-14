//! Test-only stops and sampled process measurements, not cache admission policy.
#[path = "cache_observation_tests.rs"]
mod tests;
use objc2_metal::MTLDevice;
unsafe extern "C" {
    fn mach_port_deallocate(
        task: libc::mach_port_t,
        name: libc::mach_port_t,
    ) -> libc::kern_return_t;
}
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, String>;
const GROWTH_LIMIT: u64 = 2 << 30;
#[derive(Clone, Copy, Debug)]
struct Reading {
    footprint: u64,
    resident: u64,
    metal: Option<u64>,
    swapouts: u64,
    pressure: u32,
    free: u64,
}
#[allow(deprecated)]
fn reading(device: &wgpu::Device) -> Result<Reading> {
    let mut pressure = 0u32;
    let mut size = std::mem::size_of_val(&pressure);
    let status = unsafe {
        libc::sysctlbyname(
            c"kern.memorystatus_vm_pressure_level".as_ptr(),
            (&mut pressure as *mut u32).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if status != 0 || size != std::mem::size_of_val(&pressure) || !matches!(pressure, 1 | 2 | 4) {
        return Err("unknown pressure".into());
    }
    let mut usage: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
    let status = unsafe {
        libc::proc_pid_rusage(
            libc::getpid(),
            libc::RUSAGE_INFO_V2,
            (&mut usage as *mut libc::rusage_info_v2).cast(),
        )
    };
    if status != 0 {
        return Err("process measurement unavailable".into());
    }
    let mut vm: libc::vm_statistics64_data_t = unsafe { std::mem::zeroed() };
    let mut count = libc::HOST_VM_INFO64_COUNT;
    let host = unsafe { libc::mach_host_self() };
    let status = unsafe {
        libc::host_statistics64(
            host,
            libc::HOST_VM_INFO64,
            (&mut vm as *mut libc::vm_statistics64_data_t).cast(),
            &mut count,
        )
    };
    unsafe {
        mach_port_deallocate(libc::mach_task_self_, host);
    }
    if status != libc::KERN_SUCCESS
        || (count as usize * std::mem::size_of::<libc::integer_t>())
            < std::mem::offset_of!(libc::vm_statistics64_data_t, swapouts) + 8
    {
        return Err("VM measurement unavailable".into());
    }
    let page = u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) })
        .map_err(|_| "page size unavailable")?;
    let free = u64::from(vm.free_count)
        .checked_sub(u64::from(vm.speculative_count))
        .and_then(|n| n.checked_mul(page))
        .ok_or("invalid free-page measurement")?;
    // The HAL guard bounds the read-only native getter; no resource ownership transfer.
    let metal = unsafe { device.as_hal::<wgpu::hal::api::Metal>() }
        .map(|native| native.raw_device().currentAllocatedSize() as u64);
    Ok(Reading {
        footprint: usage.ri_phys_footprint,
        resident: usage.ri_resident_size,
        metal,
        swapouts: vm.swapouts,
        pressure,
        free,
    })
}
struct Status {
    latest: Reading,
    sampled: Instant,
    peak_footprint: u64,
    peak_resident: u64,
    peak_metal: u64,
    samples: u64,
    stopped: Option<String>,
}
pub(super) struct Monitor {
    status: Mutex<Status>,
    baseline: u64,
    swapouts: u64,
}
impl Monitor {
    fn record(&self, read: Result<Reading>) {
        let mut status = self.status.lock().unwrap();
        match read {
            Err(error) => {
                status.stopped.get_or_insert(error);
            }
            Ok(read) => {
                let stop = if read.pressure != 1 {
                    Some("pressure")
                } else if read.swapouts != self.swapouts {
                    Some("reported swap counter changed")
                } else if read.footprint.saturating_sub(self.baseline) >= GROWTH_LIMIT {
                    Some("footprint growth")
                } else {
                    None
                };
                if let Some(stop) = stop {
                    status.stopped.get_or_insert(stop.into());
                }
                status.latest = read;
                status.peak_footprint = status.peak_footprint.max(read.footprint);
                status.peak_resident = status.peak_resident.max(read.resident);
                status.peak_metal = status.peak_metal.max(read.metal.unwrap_or(0));
                status.samples += 1;
                status.sampled = Instant::now();
            }
        }
    }
    pub(super) fn check(&self) -> Result<()> {
        let mut status = self.status.lock().unwrap();
        if status.sampled.elapsed() > Duration::from_secs(2) {
            status.stopped.get_or_insert("stale sampler".into());
        }
        status.stopped.as_ref().map_or(Ok(()), |reason| {
            Err(format!("cache observation stopped: {reason}"))
        })
    }
    pub(super) fn phase<T, E: std::fmt::Display>(
        &self,
        name: &str,
        action: impl FnOnce() -> std::result::Result<T, E>,
    ) -> Result<T> {
        self.check()?;
        let start = Instant::now();
        let result = action().map_err(|e| e.to_string());
        println!(
            "cache_phase name={name} seconds={:.6}",
            start.elapsed().as_secs_f64()
        );
        self.check()?;
        result
    }
}
pub(super) fn source_bound(m: &crate::raw::Metadata) -> Result<u64> {
    let bytes = u64::from(m.width)
        .checked_mul(u64::from(m.height))
        .and_then(|n| n.checked_mul(8))
        .and_then(|rgb| {
            u64::from(m.raw_width)
                .checked_mul(u64::from(m.raw_height))
                .and_then(|n| n.checked_mul(2))
                .and_then(|cfa| rgb.checked_add(cfa))
        })
        .filter(|n| *n > 0 && *n <= 600 << 20)
        .ok_or("source exceeds diagnostic geometry/600 MiB bound")?;
    Ok(bytes)
}
pub(super) fn wait(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> std::result::Result<(), wgpu::PollError> {
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(queue.submit([])),
            timeout: Some(Duration::from_secs(10)),
        })
        .map(|_| ())
}
pub(super) fn with_monitor(
    action: impl FnOnce(&Arc<Monitor>, &wgpu::Device, &wgpu::Queue) -> Result<()>,
) -> Result<()> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let (device, queue) = pollster::block_on(async {
        instance
            .request_adapter(&Default::default())
            .await
            .unwrap()
            .request_device(&Default::default())
            .await
            .unwrap()
    });
    let baseline = reading(&device)?;
    if baseline.pressure != 1 || baseline.free < GROWTH_LIMIT {
        return Err(
            "preflight requires Normal pressure and 2 GiB reported non-speculative free memory"
                .into(),
        );
    }
    let monitor = Arc::new(Monitor {
        baseline: baseline.footprint,
        swapouts: baseline.swapouts,
        status: Mutex::new(Status {
            latest: baseline,
            sampled: Instant::now(),
            peak_footprint: baseline.footprint,
            peak_resident: baseline.resident,
            peak_metal: baseline.metal.unwrap_or(0),
            samples: 1,
            stopped: None,
        }),
    });
    let done = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let sampler = scope.spawn(|| {
            while !done.load(Ordering::Relaxed) {
                monitor.record(reading(&device));
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            action(&monitor, &device, &queue)
        }));
        done.store(true, Ordering::Relaxed);
        sampler.join().unwrap();
        let status = monitor.status.lock().unwrap();
        println!(
            "cache_summary baseline_footprint={} sampled_peak_footprint={} sampled_growth={} sampled_peak_resident={} sampled_peak_device={} reported_swapouts={} pressure={} samples={} stopped={:?}",
            monitor.baseline,
            status.peak_footprint,
            status.peak_footprint.saturating_sub(monitor.baseline),
            status.peak_resident,
            status.peak_metal,
            status.latest.swapouts,
            status.latest.pressure,
            status.samples,
            status.stopped
        );
        drop(status);
        match result {
            Ok(result) => {
                result?;
                monitor.check()
            }
            Err(panic) => std::panic::resume_unwind(panic),
        }
    })
}
