use super::Presentation;
use crate::Error;

/// Vulkan on Win32 exposes no float swapchain, so presentation is SDR: 10-bit where the surface
/// offers it, 8-bit otherwise. Both formats store the pipeline's sRGB-encoded values directly,
/// so the host's copy pass needs no transfer conversion. wgpu's Vulkan surface reports only the
/// sRGB colour space, so wide-gamut or HDR presentation needs an upstream change.
pub(crate) fn presentation() -> Presentation {
    Presentation {
        color_space: wgpu::SurfaceColorSpace::Auto,
        formats: &[
            wgpu::TextureFormat::Rgb10a2Unorm,
            wgpu::TextureFormat::Bgra8Unorm,
        ],
    }
}

pub(crate) fn configure_presentation(_surface: &wgpu::Surface<'_>) -> Result<(), Error> {
    Ok(())
}

#[cfg(feature = "editor")]
pub fn install_icon() -> Result<(), Box<dyn std::error::Error>> {
    // The window icon is set per window; see `configure_window`.
    Ok(())
}

/// The taskbar and title bar take their icon from the window.
#[cfg(feature = "editor")]
pub fn configure_window(window: &winit::window::Window) -> Result<(), Box<dyn std::error::Error>> {
    let png = include_bytes!("../../assets/branding/png/icon-256.png");
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)?;
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    window.set_window_icon(Some(winit::window::Icon::from_rgba(
        rgba.into_raw(),
        width,
        height,
    )?));
    light_caption(window);
    Ok(())
}

/// The system draws the menu strip in its light colours whatever the application theme.
pub(crate) const MENU_STRIP_IS_LIGHT: bool = true;

/// Following the menu keeps one light band from the frame to the last item, instead of framing a
/// dark caption over a light menu over the dark editor.
fn light_caption(window: &winit::window::Window) {
    window.set_theme(Some(winit::window::Theme::Light));
}

/// Photo fullscreen keeps the window above the taskbar, which a borderless window sized to the
/// monitor does not do by itself.
pub(crate) struct FullscreenPresentation;

impl FullscreenPresentation {
    pub(crate) fn new(_window: &winit::window::Window) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self)
    }

    pub(crate) fn show(&self, window: &winit::window::Window, focused: bool) {
        let level = if focused {
            winit::window::WindowLevel::AlwaysOnTop
        } else {
            winit::window::WindowLevel::Normal
        };
        window.set_window_level(level);
    }
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
