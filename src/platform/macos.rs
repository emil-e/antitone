use objc2_core_graphics::{CGColorSpace, kCGColorSpaceExtendedSRGB};

use crate::Error;

#[cfg(feature = "editor")]
pub fn set_application_icon() -> Result<(), Box<dyn std::error::Error>> {
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
pub fn configure_sdr(surface: &wgpu::Surface<'_>) -> Result<(), Error> {
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
    println!(
        "presentation_layer color_space={actual} pixel_format={:?} EDR=false drawable_size={:?}",
        layer.pixelFormat(),
        layer.drawableSize()
    );
    Ok(())
}
