//! Fallible native GPU allocations. Invalid handles never become usable image resources.
use crate::Error;
use futures_util::FutureExt;

pub(crate) fn checked<T>(device: &wgpu::Device, create: impl FnOnce() -> T) -> Result<T, Error> {
    let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let value = create();
    // Native wgpu-core pops thread-local scopes with an already-ready result. Polling once
    // cannot wait for GPU completion, including when allocation happens during recording.
    let validation = validation.pop().now_or_never();
    let memory = memory.pop().now_or_never();
    match (memory, validation) {
        (Some(Some(error)), _) => Err(classify(error)),
        (_, Some(Some(error))) => Err(classify(error)),
        (Some(None), Some(None)) => Ok(value),
        _ => Err(Error::Gpu(
            "GPU allocation error reporting did not complete".into(),
        )),
    }
}

fn classify(error: wgpu::Error) -> Error {
    let details = error.to_string();
    match error {
        wgpu::Error::OutOfMemory { .. } => Error::GpuOutOfMemory(details),
        _ => Error::Gpu(details),
    }
}

pub(crate) trait DeviceAllocationExt {
    fn try_create_buffer(
        &self,
        descriptor: &wgpu::BufferDescriptor<'_>,
    ) -> Result<wgpu::Buffer, Error>;
    fn try_create_buffer_init(
        &self,
        descriptor: &wgpu::util::BufferInitDescriptor<'_>,
    ) -> Result<wgpu::Buffer, Error>;
    fn try_create_texture(
        &self,
        descriptor: &wgpu::TextureDescriptor<'_>,
    ) -> Result<wgpu::Texture, Error>;
}
impl DeviceAllocationExt for wgpu::Device {
    fn try_create_buffer(
        &self,
        descriptor: &wgpu::BufferDescriptor<'_>,
    ) -> Result<wgpu::Buffer, Error> {
        checked(self, || self.create_buffer(descriptor))
    }
    fn try_create_buffer_init(
        &self,
        descriptor: &wgpu::util::BufferInitDescriptor<'_>,
    ) -> Result<wgpu::Buffer, Error> {
        if descriptor.contents.is_empty() {
            return self.try_create_buffer(&wgpu::BufferDescriptor {
                label: descriptor.label,
                size: 0,
                usage: descriptor.usage,
                mapped_at_creation: false,
            });
        }
        let size = (descriptor.contents.len() as u64).next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT);
        let buffer = self.try_create_buffer(&wgpu::BufferDescriptor {
            label: descriptor.label,
            size,
            usage: descriptor.usage,
            mapped_at_creation: true,
        })?;
        {
            let mut mapped = buffer
                .get_mapped_range_mut(..)
                .map_err(|error| Error::Gpu(error.to_string()))?;
            mapped
                .slice(..descriptor.contents.len())
                .copy_from_slice(descriptor.contents);
        }
        buffer.unmap();
        Ok(buffer)
    }
    fn try_create_texture(
        &self,
        descriptor: &wgpu::TextureDescriptor<'_>,
    ) -> Result<wgpu::Texture, Error> {
        checked(self, || self.create_texture(descriptor))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_errors_keep_their_category_without_exhausting_the_device() {
        let error = wgpu::Error::OutOfMemory {
            source: Box::new(std::io::Error::other("injected allocation failure")),
        };
        assert!(matches!(classify(error), Error::GpuOutOfMemory(_)));
    }

    #[test]
    fn native_scopes_return_valid_allocations_and_reject_invalid_handles_without_waiting() {
        let device = pollster::block_on(async {
            let adapter = crate::gpu::instance()
                .request_adapter(&Default::default())
                .await
                .unwrap();
            let (device, _) = crate::gpu::submission::request_device(&adapter, &Default::default())
                .await
                .unwrap();
            device
        });
        let descriptor = wgpu::BufferDescriptor {
            label: None,
            size: 16,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        };
        assert_eq!(device.try_create_buffer(&descriptor).unwrap().size(), 16);
        let bad_init = wgpu::util::BufferInitDescriptor {
            label: None,
            contents: &[1, 2, 3],
            usage: wgpu::BufferUsages::empty(),
        };
        assert!(
            matches!(device.try_create_buffer_init(&bad_init), Err(Error::Gpu(_))),
            "failed initialization must return before attempting to map an invalid handle"
        );
        let init = wgpu::util::BufferInitDescriptor {
            usage: wgpu::BufferUsages::STORAGE,
            ..bad_init
        };
        assert_eq!(device.try_create_buffer_init(&init).unwrap().size(), 4);
        let invalid = wgpu::BufferDescriptor {
            size: device.limits().max_buffer_size + 4,
            ..descriptor
        };
        assert!(matches!(
            device.try_create_buffer(&invalid),
            Err(Error::Gpu(_))
        ));
        assert!(
            device.try_create_buffer(&descriptor).is_ok(),
            "a failed scope must not poison the next allocation"
        );
    }
}
