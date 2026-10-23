use crate::Error;

pub(crate) fn backend() -> wgpu::Backends {
    wgpu::Backends::DX12
}

pub(crate) fn configure_presentation(_surface: &wgpu::Surface<'_>) -> Result<(), Error> {
    Ok(())
}

#[cfg(feature = "editor")]
pub fn install_icon() -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

pub(crate) struct FullscreenPresentation;

impl FullscreenPresentation {
    pub(crate) fn new() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self)
    }

    pub(crate) fn show(&self, _focused: bool) {}
}

pub(crate) struct QuitGuard;

impl QuitGuard {
    pub(crate) fn install(
        _notify: impl Fn() -> bool + 'static,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self)
    }

    pub(crate) fn delivered(&self) -> bool {
        false
    }

    pub(crate) fn awaiting_decision(&self) -> bool {
        false
    }

    pub(crate) fn reply(&self, _allow: bool) {}

    pub(crate) fn approve_after_cleanup(&self) {}
}

pub(crate) struct QuitCancelOnUnwind;

pub(crate) fn quit_cancel_on_unwind() -> QuitCancelOnUnwind {
    QuitCancelOnUnwind
}
