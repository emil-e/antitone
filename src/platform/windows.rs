mod window_placement;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{GetMenu, HMENU, SetMenu};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{Fullscreen, Window};

use super::Presentation;
use super::host::Result as HostResult;
use crate::Error;
pub(crate) use window_placement::WindowPlacement;

/// SDR presentation uses 10-bit where offered, 8-bit otherwise. Both formats store the pipeline's
/// sRGB-encoded values directly, so the host's copy pass needs no transfer conversion. Vulkan can
/// expose scRGB and HDR10 on HDR displays; using them needs monitor-aware selection and matching
/// output encoding rather than this SDR copy path.
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

/// Reuse the caller's console for CLI output if there is one, without opening a new window.
#[cfg(feature = "editor")]
pub(super) fn prepare_process() {
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};

    // SAFETY: AttachConsole accepts the parent-process sentinel and takes no pointers.
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

#[cfg(feature = "editor")]
pub fn install_icon() -> Result<(), Box<dyn std::error::Error>> {
    // The window icon is set per window; see `configure_window`.
    Ok(())
}

pub fn configure_window(window: &winit::window::Window) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "editor")]
    install_window_icon(window)?;
    light_caption(window);
    Ok(())
}

/// The taskbar and title bar take their icon from the window.
#[cfg(feature = "editor")]
fn install_window_icon(window: &winit::window::Window) -> Result<(), Box<dyn std::error::Error>> {
    let png = include_bytes!("../../assets/branding/png/icon-256.png");
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)?;
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    window.set_window_icon(Some(winit::window::Icon::from_rgba(
        rgba.into_raw(),
        width,
        height,
    )?));
    Ok(())
}

/// The system draws the menu strip in its light colours whatever the application theme.
pub(crate) const MENU_STRIP_IS_LIGHT: bool = true;

/// Following the menu keeps one light band from the frame to the last item, instead of framing a
/// dark caption over a light menu over the dark editor.
fn light_caption(window: &winit::window::Window) {
    window.set_theme(Some(winit::window::Theme::Light));
}

pub(crate) struct FullscreenPresentation {
    hwnd: HWND,
    menu: HMENU,
}

impl FullscreenPresentation {
    pub(crate) fn enter(window: &Window) -> HostResult<Self> {
        let monitor = window
            .current_monitor()
            .ok_or("fullscreen viewing requires a display")?;
        let RawWindowHandle::Win32(handle) = window.window_handle()?.as_raw() else {
            return Err("fullscreen viewing requires a Win32 window handle".into());
        };
        let hwnd = handle.hwnd.get() as HWND;

        // A Win32 menu takes client space even without a caption. Detach only the
        // menu bar; the host still owns the menu, its subclass and its accelerators.
        // SAFETY: the host owns the live window and menu on this thread through leave.
        let menu = unsafe { GetMenu(hwnd) };
        if !menu.is_null() && unsafe { SetMenu(hwnd, std::ptr::null_mut()) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }

        // Let winit own the frame, monitor bounds, taskbar integration and placement restore.
        window.set_fullscreen(Some(Fullscreen::Borderless(Some(monitor))));
        Ok(Self { hwnd, menu })
    }

    pub(crate) fn leave(self, window: &Window) -> HostResult<()> {
        // Attach the menu before winit restores the normal window placement.
        // SAFETY: the host still owns this window and menu on the window thread.
        let menu_result = if !self.menu.is_null() && unsafe { SetMenu(self.hwnd, self.menu) } == 0 {
            Err(std::io::Error::last_os_error().into())
        } else {
            Ok(())
        };
        window.set_fullscreen(None);
        menu_result
    }

    pub(crate) fn focus_changed(&self, _focused: bool) {}
}

#[cfg(test)]
mod fullscreen_tests;

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
