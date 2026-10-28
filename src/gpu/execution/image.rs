//! Bounded uploaded camera samples, working AP1 and optional display-referred originals.
use super::raster::CameraProgram;
use crate::{Error, gpu::ProcessingSource, renderer::GpuRenderer, rendering::OutputReference};

pub(crate) struct Image {
    pub size: [u32; 2],
    pub source: ProcessingSource,
    pub working: wgpu::Buffer,
    pub texture: wgpu::Texture,
    pub binding: Option<crate::renderer::RenderBinding>,
}
impl Image {
    pub fn new(
        device: &wgpu::Device,
        renderer: &GpuRenderer,
        size: [u32; 2],
        program: CameraProgram<'_>,
        output: OutputReference,
        look: Option<&std::sync::Arc<crate::looks::LookAsset>>,
    ) -> Result<Self, Error> {
        let bytes = Self::require(device, size, output)?;
        Self::from_pixels(
            device,
            renderer,
            size,
            program,
            output,
            look,
            &vec![[0.; 4]; (bytes / 16) as usize],
        )
    }

    pub fn require(
        device: &wgpu::Device,
        size: [u32; 2],
        output: OutputReference,
    ) -> Result<u64, Error> {
        let bytes = u64::from(size[0]) * u64::from(size[1]) * 16;
        let limits = device.limits();
        if size.contains(&0)
            || bytes > limits.max_buffer_size
            || bytes > limits.max_storage_buffer_binding_size
            || (output.display_target().is_some()
                && size.iter().any(|&n| n > limits.max_texture_dimension_2d))
        {
            return Err(Error::Gpu("processing support exceeds GPU limits".into()));
        }

        Ok(bytes)
    }

    pub fn from_pixels(
        device: &wgpu::Device,
        renderer: &GpuRenderer,
        size: [u32; 2],
        program: CameraProgram<'_>,
        output: OutputReference,
        look: Option<&std::sync::Arc<crate::looks::LookAsset>>,
        pixels: &[[f32; 4]],
    ) -> Result<Self, Error> {
        let bytes = Self::require(device, size, output)?;
        if pixels.len() as u64 != bytes / 16 {
            return Err(Error::InvalidParameter(
                "original pixels differ from admitted layout",
            ));
        }
        #[cfg(all(test, feature = "editor"))]
        crate::gpu::test_support::original_image_created(size);
        let source = match program {
            CameraProgram::Interpreted(program) => program.source_continuous(device, pixels)?,
            CameraProgram::Matrix(matrix) => {
                ProcessingSource::new_continuous(device, pixels, matrix)?
            }
        };
        let working = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bounded original graded AP1 scratch"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let binding = output
            .display_target()
            .map(|target| {
                renderer.render_binding(device, &working, [size[0], size[1], 0, 0], target, look)
            })
            .transpose()?;
        let texture_size = if binding.is_some() { size } else { [1, 1] };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("bounded rendered original support"),
            size: wgpu::Extent3d {
                width: texture_size[0],
                height: texture_size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        Ok(Self {
            size,
            source,
            working,
            texture,
            binding,
        })
    }

    #[cfg(feature = "raw")]
    pub fn read(
        &self,
        scan: &crate::scan::Scan,
        read: crate::acquisition::SourceRead,
    ) -> Result<Vec<[f32; 4]>, Error> {
        let crate::acquisition::SourceRead::Originals(bounds) = read else {
            return Err(Error::InvalidParameter(
                "original adapter requires original samples",
            ));
        };
        if [bounds.width, bounds.height] != self.size {
            return Err(Error::InvalidParameter(
                "original adapter layout differs from plan",
            ));
        }
        Self::read_originals(scan, read)
    }

    #[cfg(feature = "raw")]
    pub fn read_originals(
        scan: &crate::scan::Scan,
        read: crate::acquisition::SourceRead,
    ) -> Result<Vec<[f32; 4]>, Error> {
        let crate::acquisition::SourceRead::Originals(bounds) = read else {
            return Err(Error::InvalidParameter(
                "original adapter requires original samples",
            ));
        };
        scan.tile_region(
            scan.area,
            bounds.x - scan.area.x,
            bounds.y - scan.area.y,
            bounds.width,
            bounds.height,
        )
    }

    pub fn render(
        &self,
        renderer: &GpuRenderer,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        target: crate::rendering::OutputTarget,
    ) -> Result<(), Error> {
        let binding = self.binding.as_ref().ok_or(Error::InvalidParameter(
            "display processing requires a rendering binding",
        ))?;
        binding.encode_look(renderer, device, encoder)?;
        let view = self.texture.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("per-original ACES before reconstruction"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&renderer.target(target).pipeline);
        pass.set_bind_group(0, &binding.group, &[]);
        pass.draw(0..3, 0..1);
        Ok(())
    }
}
