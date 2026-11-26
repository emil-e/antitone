use winit::window::Window;

use super::FullscreenPresentation;
use super::host::Result;

#[derive(Default)]
pub(super) struct PhotoFullscreen {
    active: Option<Active>,
}

enum Active {
    Existing,
    Owned(FullscreenPresentation),
}

impl PhotoFullscreen {
    pub(super) fn update(&mut self, window: &Window, requested: bool) -> Result<Option<bool>> {
        if requested == self.active.is_some() {
            return Ok(None);
        }

        if requested {
            self.active = Some(if window.fullscreen().is_some() {
                Active::Existing
            } else {
                Active::Owned(FullscreenPresentation::enter(window)?)
            });
        } else if let Some(Active::Owned(presentation)) = self.active.take() {
            presentation.leave(window)?;
        }

        window.request_redraw();
        Ok(Some(requested))
    }

    pub(super) fn focus_changed(&self, focused: bool) {
        if let Some(Active::Owned(presentation)) = &self.active {
            presentation.focus_changed(focused);
        }
    }
}
