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

/// Compose the concrete stores' optional allowances; active consumers keep their leases.
pub(super) fn apply_budgets(
    budgets: CacheBudgets,
    sources: &crate::source::SourceCache,
    thumbnails: &mut super::Thumbnails,
    evaluation: Option<&crate::gpu::evaluation::Context>,
) {
    sources.set_budget(budgets.sources as u64);
    let presentation_bytes = thumbnails.apply_budgets(budgets);
    if let Some(evaluation) = evaluation {
        evaluation.set_retention_limit(budgets.images.saturating_sub(presentation_bytes) as u64);
    }
}

impl super::Editor {
    pub(super) fn cache_limits(&self) -> CacheLimits {
        self.preferences
            .cache_limits
            .expect("initialized cache preferences")
    }

    pub(super) fn set_cache_limits(&mut self, limits: CacheLimits) {
        self.preferences.cache_limits = Some(limits);
        self.apply_cache_budgets();
    }

    pub(super) fn apply_cache_budgets(&mut self) {
        let budgets = CacheBudgets::new(self.cache_limits(), self.memory_pressure);
        let sources = self.document.cache.clone();
        let evaluation = self
            .source_services
            .as_ref()
            .map(super::source::Services::evaluation);
        apply_budgets(budgets, &sources, &mut self.document.thumbnails, evaluation);
    }

    fn apply_memory_pressure(
        &mut self,
        pressure: Pressure,
        cx: &crate::application::UiContext<'_, Self>,
    ) {
        self.memory_pressure = pressure;
        self.apply_cache_budgets();
        self.advance_thumbnail_demand(cx);
    }

    /// Start once at application construction; the task survives document and viewing changes.
    pub fn start_memory_watch(&mut self, cx: &crate::application::UiContext<'_, Self>) {
        if self.memory_watch.is_some() {
            return;
        }
        let (send, receive) = futures_channel::mpsc::unbounded();
        let watch = crate::sys::MemoryPressureWatch::observe(move |pressure| {
            let _ = send.unbounded_send(pressure);
        });
        self.memory_watch = cx.spawn(move |cx| async move {
            let _watch = watch;
            cx.update(|editor| editor.apply_memory_pressure(crate::sys::memory_pressure(), &cx));
            pressure_changes(receive, cx).await;
        });
    }
}

async fn pressure_changes(
    mut receive: futures_channel::mpsc::UnboundedReceiver<Pressure>,
    cx: crate::application::UiContext<'_, super::Editor>,
) {
    use futures_util::StreamExt;
    while let Some(pressure) = receive.next().await {
        if cx
            .update(|editor| editor.apply_memory_pressure(pressure, &cx))
            .is_none()
        {
            break;
        }
    }
}

#[cfg(test)]
mod tests;
