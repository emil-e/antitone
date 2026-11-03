use super::cache_limits::CacheLimits;
pub(super) use crate::sys::MemoryPressure as Pressure;

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
                gpu: crate::sys::GpuMemory::Unified,
            }
            .defaults(),
            pressure,
        )
    }
}

pub(super) struct PressureWatch {
    _watch: crate::sys::MemoryPressureWatch,
}

impl PressureWatch {
    pub fn new(context: egui::Context) -> Self {
        Self::observe(move |_| context.request_repaint())
    }

    pub(super) fn observe(notify: impl Fn(Pressure) + Send + 'static) -> Self {
        Self {
            _watch: crate::sys::MemoryPressureWatch::observe(notify),
        }
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
