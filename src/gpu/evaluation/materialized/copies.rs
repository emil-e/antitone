//! Complete transferable coverage, independent of single-shader sampling limits.
use super::*;

pub(in crate::gpu::evaluation) struct CopyCover {
    region: Rect,
    pub regions: Vec<CopyRegion>,
    pub commands: usize,
}
pub(in crate::gpu::evaluation) struct CopyRegion {
    pub layout: ImageLayout,
    pub image: Arc<ImageValue>,
    pub region: Rect,
}

impl CopyCover {
    pub fn find(
        scope: &Materializations,
        definition: &value::Value,
        region: Rect,
        commands: usize,
        remaining: &mut usize,
    ) -> Result<Option<Self>, Error> {
        let output = definition.output(region)?;
        let Some(identity) = output.identity() else {
            return Ok(None);
        };
        let mut values = Vec::new();
        for (layout, image) in scope.ready_bounded::<ImageLayout>(identity, remaining) {
            if layout.format != ImageFormat::Rgba32Float
                || !image
                    .as_buffer()
                    .is_some_and(|buffer| buffer.usage().contains(wgpu::BufferUsages::COPY_SRC))
                || overlap(layout.region, region) == 0
            {
                continue;
            }
            validate(definition.domain, *layout, &image)?;
            values.push(Part {
                layout: *layout,
                image,
            });
        }
        // A containing allocation needs no multi-piece cover search.
        if let Some(index) = values
            .iter()
            .position(|part| part.layout.region.contains_region(region))
        {
            values = vec![values.swap_remove(index)];
        }
        values.sort_by_key(|part| (part.layout.region.y, part.layout.region.x));
        let mut uncovered = vec![region];
        let mut regions = Vec::new();
        let mut count = 0usize;
        for part in values {
            let mut next = Vec::new();
            for missing in uncovered {
                let Some(left) = remaining.checked_sub(1) else {
                    return Ok(None);
                };
                *remaining = left;
                if overlap(missing, part.layout.region) > 0 {
                    let origin = [
                        missing.x.max(part.layout.region.x),
                        missing.y.max(part.layout.region.y),
                    ];
                    let end = [
                        missing.right().min(part.layout.region.right()),
                        missing.bottom().min(part.layout.region.bottom()),
                    ];
                    let covered = Rect::new(
                        origin,
                        [(end[0] - origin[0]) as u32, (end[1] - origin[1]) as u32],
                    );
                    count += covered.height as usize;
                    if count > commands {
                        return Ok(None);
                    }
                    regions.push(CopyRegion {
                        layout: part.layout,
                        image: part.image.clone(),
                        region: covered,
                    });
                }
                next.extend(subtract(missing, part.layout.region));
            }
            uncovered = next;
            if uncovered.is_empty() {
                return Ok(Some(Self {
                    region,
                    regions,
                    commands: count,
                }));
            }
        }
        Ok(None)
    }

    pub fn images(&self) -> impl Iterator<Item = &Arc<ImageValue>> {
        self.regions.iter().map(|part| &part.image)
    }

    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pixels: &wgpu::Buffer,
    ) -> Result<(), Error> {
        let target = self.region;
        if !pixels.usage().contains(wgpu::BufferUsages::COPY_DST)
            || pixels.size() < u64::from(target.width) * u64::from(target.height) * 16
        {
            return Err(Error::InvalidParameter(
                "covered image differs from copy destination",
            ));
        }
        for part in &self.regions {
            let source = part.image.as_buffer().expect("transferable covered input");
            if source == pixels {
                if part.layout.region == target {
                    continue;
                }
                return Err(Error::InvalidParameter(
                    "covered copy aliases different image regions",
                ));
            }
            for y in part.region.y..part.region.bottom() {
                encoder.copy_buffer_to_buffer(
                    source,
                    part.layout.region.index([part.region.x, y]) as u64 * 16,
                    pixels,
                    target.index([part.region.x, y]) as u64 * 16,
                    u64::from(part.region.width) * 16,
                );
            }
        }
        Ok(())
    }
}
