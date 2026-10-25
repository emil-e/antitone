use winit::dpi::{LogicalPosition, LogicalSize};
use winit::window::Window;

use super::FullscreenPresentation;
use super::host::Result;

#[derive(Default)]
pub(super) struct PhotoFullscreen {
    saved: Option<SavedWindow>,
}

struct SavedWindow {
    position: LogicalPosition<f64>,
    size: LogicalSize<f64>,
    decorated: bool,
    resized: bool,
    presentation: Option<FullscreenPresentation>,
}

impl PhotoFullscreen {
    pub(super) fn update(&mut self, window: &Window, requested: bool) -> Result<Option<bool>> {
        let mut changed = None;
        if requested && self.saved.is_none() {
            let scale = window.scale_factor();
            let resized = window.fullscreen().is_none();
            let saved = SavedWindow {
                position: window.outer_position()?.to_logical(scale),
                size: window.inner_size().to_logical(scale),
                decorated: window.is_decorated(),
                resized,
                presentation: if resized {
                    Some(FullscreenPresentation::new(window)?)
                } else {
                    None
                },
            };
            if resized {
                let monitor = window
                    .current_monitor()
                    .ok_or("fullscreen viewing requires a display")?;
                saved
                    .presentation
                    .as_ref()
                    .unwrap()
                    .show(window, window.has_focus());
                window.set_decorations(false);
                let _ = window.request_inner_size(monitor.size());
                window.set_outer_position(monitor.position());
            }
            self.saved = Some(saved);
            changed = Some(true);
            window.request_redraw();
        } else if !requested && let Some(saved) = self.saved.take() {
            if let Some(presentation) = saved.presentation.as_ref() {
                // Release the presentation (window level, menu-bar hiding) before restoring.
                presentation.show(window, false);
            }
            // Restore chrome before content size so the original outer position stays meaningful.
            if saved.resized {
                window.set_decorations(saved.decorated);
                let _ = window.request_inner_size(saved.size);
                window.set_outer_position(saved.position);
            }
            drop(saved);
            changed = Some(false);
            window.request_redraw();
        }
        self.focus_changed(window, window.has_focus());
        Ok(changed)
    }

    pub(super) fn focus_changed(&self, window: &Window, focused: bool) {
        if let Some(presentation) = self
            .saved
            .as_ref()
            .and_then(|saved| saved.presentation.as_ref())
        {
            presentation.show(window, focused);
        }
    }
}
