//! Required resident arguments for a complete graph, independent of optional retention.
use super::*;

#[cfg(all(test, feature = "editor"))]
mod tests;

#[derive(Clone)]
pub(crate) struct SuppliedImage {
    pub(super) definition: Arc<ImageNode>,
    pub(super) layout: ImageLayout,
    pub(super) image: Arc<GpuImage>,
}

impl SuppliedImage {
    pub(crate) fn from_buffer(
        definition: Arc<ImageNode>,
        region: CellRegion,
        pixels: BufferImage,
    ) -> Result<Self, Error> {
        let (layout, image) = match pixels {
            BufferImage::Produced(image) => {
                if image.as_buffer().is_none() {
                    return Err(Error::InvalidParameter(
                        "supplied image requires buffer storage",
                    ));
                }
                if definition.identity() != Some(&image.identity) {
                    return Err(Error::InvalidParameter(
                        "supplied storage belongs to a different image value",
                    ));
                }
                (*image.descriptor, image.image)
            }
            BufferImage::Submitted(pixels) => (
                ImageLayout {
                    domain: definition.domain(),
                    region,
                    format: ImageFormat::Rgba32Float,
                    backing: ImageBacking::Buffer,
                },
                GpuImage::new(pixels),
            ),
        };
        if layout.region != region {
            return Err(Error::InvalidParameter(
                "supplied storage differs from its declared region",
            ));
        }
        super::materialized::validate(definition.domain(), layout, &image)?;
        Ok(Self {
            definition,
            layout,
            image,
        })
    }

    pub(crate) fn definition(&self) -> &Arc<ImageNode> {
        &self.definition
    }

    pub(crate) fn layout(&self) -> ImageLayout {
        self.layout
    }

    pub(crate) fn covers(&self, definition: &ImageNode, region: CellRegion) -> bool {
        self.supplies(definition)
            && self.layout.domain == definition.domain()
            && self.layout.region.contains_region(region)
    }

    pub(crate) fn pixels(&self) -> &wgpu::Buffer {
        self.image.as_buffer().expect("validated supplied buffer")
    }

    pub(super) fn supplies(&self, definition: &ImageNode) -> bool {
        // The complete definition must agree; an anonymous input is equal only to itself.
        self.definition.same_definition(definition)
    }
}
