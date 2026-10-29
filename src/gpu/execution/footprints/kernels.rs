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
        let working = graded
            .stages
            .input_buffer(device, graded.source.spec.frame().output)
            .expect("validated processing input");
        let pixels = working.as_ref().unwrap_or(&graded.pixels);
        self.stages
            .encode_from(
                device,
                encoder,
                &graded.stages,
                &graded.source.pixels,
                pixels,
            )
            .expect("validated photographic composition");
        if let Some(working) = &working {
            self.stages.extract(
                device,
                encoder,
                working,
                graded.stages.raster.size(),
                &graded.pixels,
                crate::gpu::pixel_pipeline::OutputRegion {
                    size: graded.plan.output().output,
                    offset: graded
                        .plan
                        .output_offset()
                        .expect("frame-grid output extraction"),
                },
            );
        }
    }

    pub(crate) fn cover_proxy(
        &self,
        context: &crate::gpu::evaluation::Context,
        encoder: &mut wgpu::CommandEncoder,
        pixels: &wgpu::Buffer,
    ) -> Result<(), Error> {
        crate::gpu::evaluation::acquire::cover(context, encoder, pixels)
    }

    pub(crate) fn copy(
        &self,
        context: &crate::gpu::evaluation::Context,
        encoder: &mut wgpu::CommandEncoder,
        buffer: &wgpu::Buffer,
        texture: &wgpu::Texture,
    ) -> Result<(), Error> {
        crate::gpu::evaluation::acquire::copy(context, encoder, buffer, texture)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn filter(
        &self,
        context: &crate::gpu::evaluation::Context,
        encoder: &mut wgpu::CommandEncoder,
        image: &wgpu::Texture,
        output: &wgpu::Buffer,
        frame: FrameRaster,
        origin: [i32; 2],
        window: crate::resample::windows::Window,
    ) -> Result<(), Error> {
        let bounds = frame.processing_bounds()?;
        self.filter_raster(
            context,
            encoder,
            image,
            output,
            [bounds.width, bounds.height],
            frame.output,
            frame.affine_in(bounds)?,
            origin,
            window,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn filter_raster(
        &self,
        context: &crate::gpu::evaluation::Context,
        encoder: &mut wgpu::CommandEncoder,
        image: &wgpu::Texture,
        output: &wgpu::Buffer,
        source_size: [u32; 2],
        output_size: [u32; 2],
        affine: [[f32; 2]; 3],
        origin: [i32; 2],
        window: crate::resample::windows::Window,
    ) -> Result<(), Error> {
        crate::gpu::evaluation::acquire::filter(
            context,
            encoder,
            image,
            output,
            crate::gpu::evaluation::acquire::Footprint {
                source: source_size,
                output: output_size,
                affine,
                origin,
                window,
            },
        )
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
