use super::{GpuScopes, Plot};
use crate::gpu::submission::Queue;
use std::{sync::mpsc, time::Duration};

pub(crate) fn read_words(device: &wgpu::Device, queue: &Queue, source: &wgpu::Buffer) -> Vec<u32> {
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("scope test readback"),
        size: source.size(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(source, 0, &staging, 0, source.size());
    let submission = queue.submit([encoder.finish()]);
    let (send, receive) = mpsc::channel();
    staging.map_async(wgpu::MapMode::Read, .., move |result| {
        let _ = send.send(result);
    });
    let timeout = Duration::from_secs(30);
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(timeout),
        })
        .unwrap();
    receive.recv_timeout(timeout).unwrap().unwrap();
    let mapped = staging.get_mapped_range(..).unwrap();
    let words = bytemuck::cast_slice(&mapped).to_vec();
    drop(mapped);
    staging.unmap();
    words
}

pub(crate) fn sample_count(
    device: &wgpu::Device,
    queue: &Queue,
    scopes: &GpuScopes,
    plot: Plot,
) -> u32 {
    let totals = read_words(device, queue, &scopes.totals);
    if plot == Plot::Vector {
        totals[0]
    } else {
        totals[1..].iter().sum()
    }
}
