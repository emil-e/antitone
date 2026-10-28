use super::*;
use egui_wgpu::CallbackTrait;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) struct SelectionTiming {
    pub id: u64,
    pub started: std::time::Instant,
    pending_painted: AtomicBool,
    ready_painted: AtomicBool,
}

impl SelectionTiming {
    #[cfg(all(test, feature = "ui-tests"))]
    pub(super) fn pending_was_painted(&self) -> bool {
        self.pending_painted.load(Ordering::Relaxed)
    }

    #[cfg(all(test, feature = "ui-tests"))]
    pub(super) fn ready_was_painted(&self) -> bool {
        self.ready_painted.load(Ordering::Relaxed)
    }

    pub fn new(id: u64, started: std::time::Instant) -> Self {
        Self {
            id,
            started,
            pending_painted: AtomicBool::new(false),
            ready_painted: AtomicBool::new(false),
        }
    }
}

pub(super) struct SelectionPaint {
    image: ImagePaint,
    timing: Option<Arc<SelectionTiming>>,
    pending: bool,
}

impl SelectionPaint {
    pub fn new(image: ImagePaint, timing: Option<Arc<SelectionTiming>>, pending: bool) -> Self {
        Self {
            image,
            timing,
            pending,
        }
    }
}

impl CallbackTrait for SelectionPaint {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        self.image
            .prepare(device, queue, screen, encoder, resources)
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        self.image.paint(info, pass, resources);

        if let Some(timing) = &self.timing {
            let painted = if self.pending {
                &timing.pending_painted
            } else {
                &timing.ready_painted
            };
            if !painted.swap(true, Ordering::Relaxed) {
                log::debug!(
                    request = timing.id,
                    stage = if self.pending { "pending" } else { "ready" },
                    boundary = "paint_callback",
                    elapsed_ms = timing.started.elapsed().as_secs_f64() * 1000.;
                    "selected_switch"
                );
            }
        }
    }
}

impl Editor {
    pub(super) fn validate_pending_preview(&mut self) {
        if self
            .document
            .visit
            .pending_preview
            .as_ref()
            .is_some_and(|preview| {
                self.document.visit.working_view.displayed.is_some()
                    || self.document_pending.is_some()
                    || self.document.picking.kind.is_some()
                    || self.document.flat_edit.is_some()
                    || self.document.visit.crop_edit.is_some()
                    || !preview.current_source(
                        &self.document,
                        self.document.visit.current_source,
                        self.document.correction.as_ref(),
                    )
            })
        {
            self.document.visit.pending_preview = None;
        }
    }

    pub(super) fn selected_view(&self) -> Option<(Rect, crate::viewer::GpuProxy, bool)> {
        if let Some(ready) = &self.document.visit.ready
            && (self.document.visit.working_view.displayed.is_some()
                || self.document.visit.pending_preview.is_none())
        {
            return Some((ready.area, ready.proxy.clone(), false));
        }

        self.document
            .visit
            .pending_preview
            .as_ref()
            .map(|preview| (preview.area(), preview.proxy.clone(), true))
            .or_else(|| {
                self.document
                    .visit
                    .ready
                    .as_ref()
                    .map(|ready| (ready.area, ready.proxy.clone(), false))
            })
    }
}
