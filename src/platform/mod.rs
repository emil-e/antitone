#[cfg(target_os = "macos")]
mod fullscreen;
#[cfg(target_os = "macos")]
pub mod host;
#[cfg(target_os = "macos")]
pub mod macos;

mod submission;
pub use submission::SubmissionGate;

pub fn device_limits(supported: wgpu::Limits) -> wgpu::Limits {
    wgpu::Limits {
        max_storage_buffer_binding_size: supported.max_storage_buffer_binding_size,
        max_buffer_size: supported.max_buffer_size,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn device_limits_raise_only_buffer_capacities_to_supported_values() {
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
                ..Default::default()
            };
            assert_eq!(super::device_limits(supported), expected);
        }
    }
}
