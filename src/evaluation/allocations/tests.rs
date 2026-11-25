use super::*;

#[test]
fn physical_storage_is_counted_once_across_distinct_image_wrappers() {
    let (device, _, _) = crate::photographic::test_support::render_context();
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 4096,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 8,
            height: 4,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let mut allocations = Allocations::default();
    allocations.buffer(&buffer);
    allocations.image(&ImageValue::new(buffer.clone()));
    allocations.image(&ImageValue::new(buffer));
    allocations.image(&ImageValue::texture(texture.clone()));
    allocations.image(&ImageValue::texture(texture));
    let host = ImageValue::host(vec![[0.; 4]; 17]);
    allocations.image(&host);
    allocations.image(&host);
    assert_eq!(allocations.bytes(), 4096 + 8 * 4 * 16 + 17 * 16);
}
