mod fullscreen;
pub mod host;
mod latency;
pub mod menu;

#[cfg(target_os = "macos")]
mod termination;

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod native;
#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod native;
#[cfg(not(any(target_os = "macos", windows)))]
compile_error!(
    "desktop platform support is implemented for macOS and Windows; add a Linux capability module with the same exports"
);

pub(crate) use native::{
    FullscreenPresentation, MENU_STRIP_IS_LIGHT, QuitGuard, configure_presentation, presentation,
    quit_cancel_on_unwind,
};
#[cfg(feature = "editor")]
pub use native::{configure_window, install_icon};

pub use crate::gpu::SubmissionGate;

/// The surface formats a platform can present the shared sRGB-encoded output in, best first,
/// and the colour space those values are interpreted in.
pub(crate) struct Presentation {
    pub(crate) color_space: wgpu::SurfaceColorSpace,
    pub(crate) formats: &'static [wgpu::TextureFormat],
}

pub fn device_limits(supported: wgpu::Limits) -> wgpu::Limits {
    wgpu::Limits {
        max_storage_buffer_binding_size: supported.max_storage_buffer_binding_size,
        max_buffer_size: supported.max_buffer_size,
        max_texture_dimension_2d: supported.max_texture_dimension_2d,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn device_limits_raise_only_view_storage_capacities_to_supported_values() {
        for (binding, buffer) in [
            (128 << 20, 256 << 20),
            (1 << 30, 1 << 30),
            ((1 << 32) - 4, (1 << 32) - 1),
        ] {
            let supported = wgpu::Limits {
                max_storage_buffer_binding_size: binding,
                max_buffer_size: buffer,
                max_texture_dimension_2d: 32768,
                ..Default::default()
            };
            let expected = wgpu::Limits {
                max_storage_buffer_binding_size: binding,
                max_buffer_size: buffer,
                max_texture_dimension_2d: 32768,
                ..Default::default()
            };
            assert_eq!(super::device_limits(supported), expected);
        }
    }
}
