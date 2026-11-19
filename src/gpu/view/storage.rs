use crate::Error;
use crate::gpu::allocation::DeviceAllocationExt;

pub fn uniform(device: &wgpu::Device, bytes: &[u8]) -> Result<wgpu::Buffer, Error> {
    device.try_create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytes,
        usage: wgpu::BufferUsages::UNIFORM,
    })
}

#[cfg(test)]
pub fn view_bytes(device: &wgpu::Device, size: [u32; 2]) -> Result<u64, Error> {
    let bytes = u64::from(size[0])
        .checked_mul(u64::from(size[1]))
        .and_then(|n| n.checked_mul(16))
        .ok_or_else(|| Error::Gpu("physical-view size overflow".into()))?;
    if size.contains(&0)
        || size
            .iter()
            .any(|v| *v > device.limits().max_texture_dimension_2d)
        || bytes > device.limits().max_storage_buffer_binding_size
        || bytes > device.limits().max_buffer_size
    {
        return Err(Error::Gpu("physical view exceeds GPU limits".into()));
    }
    Ok(bytes)
}

#[cfg(test)]
pub fn float_buffer(device: &wgpu::Device, size: [u32; 2]) -> Result<wgpu::Buffer, Error> {
    let bytes = view_bytes(device, size)?;
    device.try_create_buffer(&wgpu::BufferDescriptor {
        label: Some("physical-view signed/HDR AP1"),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
