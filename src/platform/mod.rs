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

#[cfg(feature = "editor")]
pub use native::install_icon;
pub(crate) use native::{
    FullscreenPresentation, QuitGuard, backend, configure_presentation, quit_cancel_on_unwind,
};

pub use crate::gpu::SubmissionGate;

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
