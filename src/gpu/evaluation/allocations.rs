//! Physical allocation accounting for live products, independent of retention decisions.

use super::*;

#[cfg(test)]
mod tests;

#[derive(Default)]
pub(crate) struct Allocations {
    buffers: Vec<wgpu::Buffer>,
    textures: Vec<(wgpu::Texture, u64)>,
    host: Vec<Arc<[[f32; 4]]>>,
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
