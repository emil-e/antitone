use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSApplicationPresentationOptions};
use objc2_core_graphics::{CGColorSpace, kCGColorSpaceExtendedSRGB};

use crate::Error;

use super::Presentation;
pub(crate) use super::termination::{
    Guard as QuitGuard, cancel_on_unwind as quit_cancel_on_unwind,
};

/// The tagged extended sRGB layer consumes the pipeline's encoded values directly.
pub(crate) fn presentation() -> Presentation {
    Presentation {
        color_space: wgpu::SurfaceColorSpace::Auto,
        formats: &[wgpu::TextureFormat::Rgba16Float],
    }
}

pub(crate) fn configure_presentation(surface: &wgpu::Surface<'_>) -> Result<(), Error> {
    configure_sdr(surface)
}

#[cfg(feature = "editor")]
pub fn install_icon() -> Result<(), Box<dyn std::error::Error>> {
    set_application_icon()
}

/// The application icon is set through AppKit; the window carries no icon of its own.
#[cfg(feature = "editor")]
pub fn configure_window(_window: &winit::window::Window) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

#[cfg(feature = "editor")]
fn set_application_icon() -> Result<(), Box<dyn std::error::Error>> {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let main = MainThreadMarker::new().ok_or("application icon requires the main thread")?;
    let data = NSData::with_bytes(include_bytes!("../../assets/branding/icon.icns"));
    let image = NSImage::initWithData(NSImage::alloc(), &data)
        .ok_or("cannot decode the embedded application icon")?;
    // The host initializes the editor on AppKit's main thread; AppKit retains the image.
    unsafe { NSApplication::sharedApplication(main).setApplicationIconImage(Some(&image)) };
    Ok(())
}

/// Reapply after every configuration: wgpu's SDR configuration clears the layer color space.
fn configure_sdr(surface: &wgpu::Surface<'_>) -> Result<(), Error> {
    // The HAL guard and layer lock prevent concurrent surface access while setting native properties.
    let native = unsafe { surface.as_hal::<wgpu::hal::api::Metal>() }
        .ok_or_else(|| Error::Gpu("managed presentation requires the Metal backend".into()))?;
    let layer = native.render_layer().lock();
    // The named CoreGraphics constant is available on the deployment baseline, macOS 14+.
    let name = unsafe { kCGColorSpaceExtendedSRGB };
    let space = CGColorSpace::with_name(Some(name))
        .ok_or_else(|| Error::Gpu("extended sRGB color space unavailable".into()))?;
    layer.setColorspace(Some(&space));
    layer.setWantsExtendedDynamicRangeContent(false);
    let actual = layer
        .colorspace()
        .and_then(|space| CGColorSpace::name(Some(&space)))
        .ok_or_else(|| Error::Gpu("Metal layer is untagged".into()))?;
    if actual.to_string() != name.to_string() || layer.wantsExtendedDynamicRangeContent() {
        return Err(Error::Gpu(
            "Metal layer SDR/color-space configuration did not persist".into(),
        ));
    }
    log::debug!(
        color_space = actual.to_string().as_str(),
        pixel_format:? = layer.pixelFormat(),
        edr = false,
        drawable_size:? = layer.drawableSize();
        "presentation_layer"
    );
    Ok(())
}

pub(crate) struct FullscreenPresentation {
    app: Retained<NSApplication>,
    previous: NSApplicationPresentationOptions,
}

impl FullscreenPresentation {
    pub(crate) fn new(_window: &winit::window::Window) -> Result<Self, Box<dyn std::error::Error>> {
        let main = MainThreadMarker::new().ok_or("fullscreen viewing requires the main thread")?;
        let app = NSApplication::sharedApplication(main);
        let previous = app.presentationOptions();
        Ok(Self { app, previous })
    }

    pub(crate) fn show(&self, _window: &winit::window::Window, focused: bool) {
        use NSApplicationPresentationOptions as Options;
        // These paired auto-hide options preserve system access and do not create a fullscreen Space.
        let desired = if focused {
            Options::AutoHideDock | Options::AutoHideMenuBar
        } else {
            self.previous
        };
        if self.app.presentationOptions() != desired {
            self.app.setPresentationOptions(desired);
        }
    }
}

impl Drop for FullscreenPresentation {
    fn drop(&mut self) {
        self.app.setPresentationOptions(self.previous);
    }
}
