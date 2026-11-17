//! Physical allocation accounting for live products, independent of retention decisions.

use super::*;

#[cfg(test)]
mod tests;

#[derive(Clone, Default)]
pub(crate) struct Allocations {
    buffers: Vec<wgpu::Buffer>,
    textures: Vec<(wgpu::Texture, u64)>,
    host: Vec<Arc<[[f32; 4]]>>,
    images: Vec<Arc<ImageValue>>,
    views: Vec<wgpu::TextureView>,
    samplers: Vec<wgpu::Sampler>,
}

impl Allocations {
    pub(crate) fn buffer(&mut self, buffer: &wgpu::Buffer) {
        if !self.buffers.contains(buffer) {
            self.buffers.push(buffer.clone());
        }
    }

    pub(super) fn texture(&mut self, texture: &wgpu::Texture, bytes: u64) {
        if !self.textures.iter().any(|(stored, _)| stored == texture) {
            self.textures.push((texture.clone(), bytes));
        }
    }

    pub(crate) fn image(&mut self, image: &Arc<ImageValue>) {
        if !self.images.iter().any(|held| Arc::ptr_eq(held, image)) {
            self.images.push(image.clone());
        }
        if let Some(buffer) = image.as_buffer() {
            self.buffer(buffer);
        } else if let Some(texture) = image.as_texture() {
            let bytes = u64::from(texture.width())
                * u64::from(texture.height())
                * u64::from(texture.depth_or_array_layers())
                * u64::from(
                    texture
                        .format()
                        .block_copy_size(None)
                        .expect("image pixel format"),
                );
            self.texture(texture, bytes);
        } else if let materializations::ImageStorage::Host(pixels) = &image.storage
            && !self.host.iter().any(|stored| Arc::ptr_eq(stored, pixels))
        {
            self.host.push(pixels.clone());
        }
    }

    pub(crate) fn buffered(&mut self, image: &BufferImage) {
        self.buffer(image);
        if let BufferImage::Produced(image) = image {
            for input in image.inputs.iter() {
                self.image(input);
            }
        }
    }

    pub(super) fn shared(&mut self, resource: &Shared) {
        match resource {
            Shared::Buffer(buffer) => self.buffer(buffer),
            Shared::Image { image, .. } => self.image(image),
            Shared::Texture(view) | Shared::StorageTexture(view) => {
                let texture = view.texture();
                let (block_width, block_height) = texture.format().block_dimensions();
                let block_bytes = u64::from(texture.format().block_copy_size(None).unwrap_or(16));
                let bytes = (0..texture.mip_level_count())
                    .map(|level| {
                        let width = (texture.width() >> level).max(1).div_ceil(block_width);
                        let height = (texture.height() >> level).max(1).div_ceil(block_height);
                        let depth = if texture.dimension() == wgpu::TextureDimension::D3 {
                            (texture.depth_or_array_layers() >> level).max(1)
                        } else {
                            texture.depth_or_array_layers()
                        };
                        u64::from(width)
                            * u64::from(height)
                            * u64::from(depth)
                            * u64::from(texture.sample_count())
                            * block_bytes
                    })
                    .sum();
                self.texture(texture, bytes);
                if !self.views.contains(view) {
                    self.views.push(view.clone());
                }
            }
            Shared::Sampler(sampler) => {
                if !self.samplers.contains(sampler) {
                    self.samplers.push(sampler.clone());
                }
            }
        }
    }

    pub(super) fn extend(&mut self, other: &Self) {
        for buffer in &other.buffers {
            self.buffer(buffer);
        }
        for (texture, bytes) in &other.textures {
            self.texture(texture, *bytes);
        }
        for image in &other.images {
            self.image(image);
        }
        for view in &other.views {
            if !self.views.contains(view) {
                self.views.push(view.clone());
            }
        }
        for sampler in &other.samplers {
            if !self.samplers.contains(sampler) {
                self.samplers.push(sampler.clone());
            }
        }
    }

    #[cfg(test)]
    pub(super) fn owns(&self, image: &Arc<ImageValue>) -> bool {
        image
            .as_buffer()
            .is_some_and(|buffer| self.buffers.contains(buffer))
            || image
                .as_texture()
                .is_some_and(|texture| self.textures.iter().any(|(held, _)| held == texture))
            || self.images.iter().any(|held| Arc::ptr_eq(held, image))
    }

    pub(super) fn bytes_except(&self, active: &Self) -> u64 {
        self.buffers
            .iter()
            .filter(|buffer| !active.buffers.contains(buffer))
            .map(wgpu::Buffer::size)
            .sum::<u64>()
            + self
                .textures
                .iter()
                .filter(|(texture, _)| !active.textures.iter().any(|(held, _)| held == texture))
                .map(|(_, bytes)| bytes)
                .sum::<u64>()
    }

    pub(crate) fn bytes(&self) -> u64 {
        self.buffers.iter().map(wgpu::Buffer::size).sum::<u64>()
            + self.textures.iter().map(|(_, bytes)| bytes).sum::<u64>()
            + self
                .host
                .iter()
                .map(|pixels| pixels.len() as u64 * 16)
                .sum::<u64>()
    }
}
