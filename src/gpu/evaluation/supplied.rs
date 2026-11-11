//! Required resident arguments for a complete graph, independent of optional retention.
use super::*;

#[cfg(all(test, feature = "editor"))]
mod tests;

#[derive(Clone)]
pub(crate) struct SuppliedImage {
    pub(super) definition: value::Value,
    output: Output,
    pub(super) layout: ImageLayout,
    pub(super) image: Arc<ImageValue>,
}

impl SuppliedImage {
    pub(crate) fn from_buffer(
        definition: impl Into<value::Value>,
        region: CellRegion,
        pixels: BufferImage,
    ) -> Result<Self, Error> {
        let definition = definition.into();
        let output = definition.output(region)?;
        let (layout, image) = match pixels {
            BufferImage::Produced(image) => {
                if image.as_buffer().is_none() {
                    return Err(Error::InvalidParameter(
                        "supplied image requires buffer storage",
                    ));
                }
                if output.identity() != Some(&image.identity) {
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
                ImageValue::new(pixels),
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
            output,
            layout,
            image,
        })
    }

    pub(crate) fn definition(&self) -> &value::Value {
        &self.definition
    }

    pub(crate) fn layout(&self) -> ImageLayout {
        self.layout
    }

    pub(crate) fn covers(&self, definition: impl Into<value::Value>, region: CellRegion) -> bool {
        let definition = definition.into();
        self.supplies(&definition)
            && self.layout.domain == definition.domain()
            && self.layout.region.contains_region(region)
    }

    pub(crate) fn pixels(&self) -> &wgpu::Buffer {
        self.image.as_buffer().expect("validated supplied buffer")
    }

    pub(super) fn supplies(&self, definition: &value::Value) -> bool {
        definition
            .output(self.layout.region)
            .is_ok_and(|output| self.output.same_value(&output))
    }
}
