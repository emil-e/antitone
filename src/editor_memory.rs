pub(super) use crate::resources::MemoryPressure as Pressure;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CacheBudgets {
    pub sources: usize,
    pub images: usize,
    pub pressure: Pressure,
}
impl CacheBudgets {
    pub fn for_memory(bytes: u64, pressure: Pressure) -> Self {
        let mib = 1024 * 1024;
        let sources = (bytes / 1024).clamp(16 * mib, 64 * mib) as usize;
        let images = (bytes / 256).clamp(32 * mib, 256 * mib) as usize;
        let (sources, images) = match pressure {
            Pressure::Normal => (sources, images),
            Pressure::Warning => (sources / 4, images / 4),
            Pressure::Critical => (0, 0),
        };
        Self {
            sources,
            images,
            pressure,
        }
    }
    pub fn current() -> Self {
        let memory = crate::resources::physical_memory();
        let pressure = crate::resources::memory_pressure();
        Self::for_memory(memory, pressure)
    }
}
pub(super) struct PressureWatch {
    _alive: std::sync::Arc<()>,
}
impl PressureWatch {
    pub fn new(context: egui::Context) -> Self {
        let alive = std::sync::Arc::new(());
        let weak = std::sync::Arc::downgrade(&alive);
        let mut previous = CacheBudgets::current().pressure;
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
                if weak.upgrade().is_none() {
                    break;
                }
                let current = CacheBudgets::current().pressure;
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
    fn optional_budgets_use_available_machine_capacity_and_shrink_on_pressure() {
        let small = CacheBudgets::for_memory(8 << 30, Pressure::Normal);
        let large = CacheBudgets::for_memory(48 << 30, Pressure::Normal);
        assert!(large.images > small.images);
        assert!(large.sources > small.sources);
        assert!(large.sources >= 36 * 256 * 256 * 8);
        assert!(large.images >= 72 * 256 * 256 * 40);
        let warning = CacheBudgets::for_memory(48 << 30, Pressure::Warning);
        assert_eq!(warning.images, large.images / 4);
        let critical = CacheBudgets::for_memory(48 << 30, Pressure::Critical);
        assert_eq!((critical.sources, critical.images), (0, 0));
    }
}
