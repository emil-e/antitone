//! Device-local heap capacity and live resource allocations are separate measurements.
#[cfg(feature = "editor")]
use super::GpuMemory;
#[cfg(feature = "editor")]
use ash::vk;

/// Bytes attributed to live Vulkan buffers/textures, including deferred destruction, but excluding
/// unused allocator blocks and driver residency. Reading counters avoids walking every allocation
/// in the frame-latency path.
#[cfg(any(test, feature = "ui"))]
pub(crate) fn allocated_resource_bytes(device: &wgpu::Device) -> Option<usize> {
    // HAL access is read-only; the guard keeps the device alive for the backend check.
    let _native = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }?;
    let counters = device.get_internal_counters().hal;
    usize::try_from(counters.buffer_memory.read())
        .ok()?
        .checked_add(usize::try_from(counters.texture_memory.read()).ok()?)
}

#[cfg(feature = "editor")]
pub(super) fn memory_capacity(device: &wgpu::Device) -> GpuMemory {
    // The guard owns the lifetime of both instance and physical-device handles. Property queries
    // do not mutate device state or require queue synchronization.
    let Some(native) = (unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }) else {
        return GpuMemory::Unknown;
    };
    let instance = native.shared_instance().raw_instance();
    let physical = native.raw_physical_device();
    let properties = unsafe { instance.get_physical_device_properties(physical) };
    let memory = unsafe { instance.get_physical_device_memory_properties(physical) };
    classify_capacity(properties.device_type, memory.memory_heaps_as_slice())
}

#[cfg(feature = "editor")]
fn classify_capacity(kind: vk::PhysicalDeviceType, heaps: &[vk::MemoryHeap]) -> GpuMemory {
    match kind {
        vk::PhysicalDeviceType::INTEGRATED_GPU => GpuMemory::Unified,
        vk::PhysicalDeviceType::DISCRETE_GPU => {
            // Count heaps, not memory types: several types can refer to the same physical heap.
            // Static local capacity is the basis for cache defaults, not a changing budget or usage.
            let bytes = heaps
                .iter()
                .filter(|heap| heap.flags.contains(vk::MemoryHeapFlags::DEVICE_LOCAL))
                .try_fold(0u64, |total, heap| total.checked_add(heap.size));
            GpuMemory::Discrete {
                dedicated_bytes: bytes.filter(|&bytes| bytes != 0),
            }
        }
        _ => GpuMemory::Unknown,
    }
}

#[cfg(all(test, feature = "editor"))]
mod tests {
    use super::*;

    #[test]
    fn capacity_counts_local_heaps_not_shared_system_memory() {
        let local = |size| vk::MemoryHeap {
            size,
            flags: vk::MemoryHeapFlags::DEVICE_LOCAL,
        };
        let heaps = [
            local(8 << 30),
            vk::MemoryHeap {
                size: 32 << 30,
                flags: vk::MemoryHeapFlags::empty(),
            },
            local(2 << 30),
        ];
        assert_eq!(
            classify_capacity(vk::PhysicalDeviceType::DISCRETE_GPU, &heaps),
            GpuMemory::Discrete {
                dedicated_bytes: Some(10 << 30)
            }
        );
        assert_eq!(
            classify_capacity(vk::PhysicalDeviceType::INTEGRATED_GPU, &heaps),
            GpuMemory::Unified
        );
        for kind in [vk::PhysicalDeviceType::CPU, vk::PhysicalDeviceType::OTHER] {
            assert_eq!(classify_capacity(kind, &heaps), GpuMemory::Unknown);
        }
        for heaps in [&[][..], &[local(0)], &[local(u64::MAX), local(1)]] {
            assert_eq!(
                classify_capacity(vk::PhysicalDeviceType::DISCRETE_GPU, heaps),
                GpuMemory::Discrete {
                    dedicated_bytes: None
                }
            );
        }
        assert_eq!(super::super::gpu_memory(None), GpuMemory::Unknown);
    }

    #[test]
    fn live_vulkan_queries_report_capacity_and_resource_retirement() {
        let instance = crate::gpu::instance();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
            .expect("a Vulkan adapter is required");
        let info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))
            .expect("a Vulkan device is required");
        let capacity = memory_capacity(&device);
        match info.device_type {
            wgpu::DeviceType::DiscreteGpu => assert!(matches!(
                capacity,
                GpuMemory::Discrete { dedicated_bytes: Some(bytes) } if bytes > 0
            )),
            wgpu::DeviceType::IntegratedGpu => assert_eq!(capacity, GpuMemory::Unified),
            _ => assert_eq!(capacity, GpuMemory::Unknown),
        }
        // Device creation stages internal initialization data. Flush it before measuring a stable
        // baseline so its retirement is not mistaken for retirement of our test allocations.
        queue.submit([]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let baseline = allocated_resource_bytes(&device).expect("Vulkan allocation counters");
        let size = 4 * 1024 * 1024;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("memory-query lifetime check"),
            size,
            usage: wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("memory-query texture check"),
            size: wgpu::Extent3d {
                width: 512,
                height: 512,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let allocated = allocated_resource_bytes(&device).unwrap();
        assert!(allocated >= baseline + size as usize + 512 * 512 * 4);
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.clear_buffer(&buffer, 0, None);
        let commands = encoder.finish();
        let retained = buffer.clone();
        drop(buffer);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert_eq!(allocated_resource_bytes(&device), Some(allocated));
        drop(retained);
        drop(texture);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert!(
            allocated_resource_bytes(&device).unwrap() >= baseline + size as usize,
            "encoded work retains the buffer after its application handles are dropped"
        );
        queue.submit([commands]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let retired = allocated_resource_bytes(&device).unwrap();
        assert_eq!(
            retired, baseline,
            "unused allocator reservation is not live resources"
        );
        eprintln!(
            "GPU memory query: {info:?}; capacity={capacity:?}; baseline={baseline}; allocated={allocated}; retired={retired}"
        );
    }
}
