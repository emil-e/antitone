use super::processing::Processed;
use super::*;

pub(crate) struct Kernels {
    pub(crate) stages: crate::gpu::pixel_pipeline::Kernels,
}

impl Kernels {
    pub async fn new(device: &wgpu::Device) -> Result<Arc<Self>, Error> {
        Ok(Arc::new(Self {
            stages: crate::gpu::pixel_pipeline::Kernels::new(device).await?,
        }))
    }

    pub(crate) fn grade(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        graded: &Processed,
    ) {
        let Some(stages) = &graded.stages else {
            return;
        };
        debug_assert_eq!(stages.raster.size(), graded.plan.input().size());
        self.stages
            .encode_from(
                device,
                encoder,
                stages,
                &graded.source.pixels,
                &graded.pixels,
            )
            .expect("validated photographic composition");
    }
}

pub(crate) fn uniform(device: &wgpu::Device, bytes: &[u8]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytes,
        usage: wgpu::BufferUsages::UNIFORM,
    })
}

pub(crate) fn view_bytes(device: &wgpu::Device, size: [u32; 2]) -> Result<u64, Error> {
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

pub(crate) fn float_buffer(device: &wgpu::Device, size: [u32; 2]) -> Result<wgpu::Buffer, Error> {
    let bytes = view_bytes(device, size)?;
    Ok(device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("physical-view signed/HDR AP1"),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    }))
}
