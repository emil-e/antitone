use super::cache_limits::CacheLimits;
pub(super) use crate::resources::MemoryPressure as Pressure;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CacheBudgets {
    pub sources: usize,
    pub images: usize,
    pub pressure: Pressure,
}

impl CacheBudgets {
    pub fn new(limits: CacheLimits, pressure: Pressure) -> Self {
        assert!(limits.valid());
        // Emergency backoff drops optional ownership, without rewriting chosen preferences.
        let (sources, images) = match pressure {
            Pressure::Normal => (limits.cpu_bytes as usize, limits.gpu_bytes as usize),
            Pressure::Warning | Pressure::Critical => (0, 0),
        };
        Self {
            sources,
            images,
            pressure,
        }
    }

    #[cfg(test)]
    pub fn for_memory(bytes: u64, pressure: Pressure) -> Self {
        Self::new(
            super::cache_limits::Hardware {
                ram_bytes: Some(bytes),
                gpu: super::cache_limits::GpuMemory::Unified,
            }
            .defaults(),
            pressure,
        )
    }
}

pub(super) struct PressureWatch {
    _alive: std::sync::Arc<()>,
}

impl PressureWatch {
    pub fn new(context: egui::Context) -> Self {
        let alive = std::sync::Arc::new(());
        let weak = std::sync::Arc::downgrade(&alive);
        let mut previous = crate::resources::memory_pressure();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
                if weak.upgrade().is_none() {
                    break;
                }
                let current = crate::resources::memory_pressure();
                if current != previous {
                    previous = current;
                    context.request_repaint();
                }
            }
        });
        Self { _alive: alive }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressure_stops_optional_retention_without_changing_or_increasing_limits() {
        let limits = CacheLimits {
            cpu_bytes: 1024 << 20,
            gpu_bytes: 256 << 20,
        };
        let normal = CacheBudgets::new(limits, Pressure::Normal);
        for pressure in [Pressure::Warning, Pressure::Critical] {
            let backoff = CacheBudgets::new(limits, pressure);
            assert_eq!((backoff.sources, backoff.images), (0, 0));
        }
        assert_eq!(CacheBudgets::new(limits, Pressure::Normal), normal);
        assert_eq!(limits.cpu_bytes, 1024 << 20);
    }
}
