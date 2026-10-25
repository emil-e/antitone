use super::GpuImage;

pub(crate) fn image_pixels(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    image: &GpuImage,
) -> Vec<[f32; 4]> {
    assert_eq!(image.linear.format(), wgpu::TextureFormat::Rgba32Float);
    let width = image.linear.width();
    let height = image.linear.height();
    let stride = (width * 16).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("target-linear test readback"),
        size: u64::from(stride) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        image.linear.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(height),
            },
        },
        image.linear.size(),
    );
    let submission = queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    buffer.map_async(wgpu::MapMode::Read, .., move |result| {
        send.send(result).unwrap()
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .unwrap();
    receive.recv().unwrap().unwrap();
    let mapped = buffer.get_mapped_range(..).unwrap();
    let pixels: &[[f32; 4]] = bytemuck::cast_slice(&mapped);
    let result = pixels
        .chunks(stride as usize / 16)
        .flat_map(|row| row[..width as usize].iter().copied())
        .collect();
    drop(mapped);
    buffer.unmap();
    result
}
