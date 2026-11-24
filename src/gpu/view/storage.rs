use crate::Error;
use crate::gpu::allocation::DeviceAllocationExt;

pub fn uniform(device: &wgpu::Device, bytes: &[u8]) -> Result<wgpu::Buffer, Error> {
    device.try_create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytes,
        usage: wgpu::BufferUsages::UNIFORM,
    })
}
