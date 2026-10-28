//! Bounded original-cell processing followed by consumer-owned reconstruction.
use super::*;
#[cfg(test)]
mod tests;
use crate::{
    frame_raster::FrameRaster,
    geometry::{Composition, FramePoint},
};

pub struct OriginalBatch {
    source: SourceRegion,
    output: SourceRegion,
    pipeline: PixelPipeline,
    tile: [u32; 2],
}

pub struct OriginalUnit {
    pub plan: Plan,
    pub output: SourceRegion,
    pub offset: [u32; 2],
}

impl OriginalBatch {
    pub fn new(
        source: SourceRegion,
        output: SourceRegion,
        pipeline: &PixelPipeline,
        tile: [u32; 2],
    ) -> Result<Self, Error> {
        source.validate()?;
        output.validate()?;
        if !source.contains_region(output) || tile.contains(&0) {
            return Err(Error::InvalidParameter(
                "invalid original processing demand",
            ));
        }
        Ok(Self {
            source,
            output,
            pipeline: pipeline.clone(),
            tile,
        })
    }

    pub fn units(&self) -> impl Iterator<Item = Result<OriginalUnit, Error>> + '_ {
        let mut next = [0; 2];
        std::iter::from_fn(move || {
            if next[1] >= self.output.height {
                return None;
            }
            let output: SourceRegion = [
                self.output.x + next[0],
                self.output.y + next[1],
                self.tile[0].min(self.output.width - next[0]),
                self.tile[1].min(self.output.height - next[1]),
            ]
            .into();
            next[0] = next[0].saturating_add(self.tile[0]);
            if next[0] >= self.output.width {
                next[0] = 0;
                next[1] = next[1].saturating_add(self.tile[1]);
            }
            Some((|| {
                let composition =
                    Composition::new(self.source, Default::default(), Default::default())?;
                let min = [output.x - self.source.x, output.y - self.source.y].map(f64::from);
                let frame = FrameRaster::new(
                    composition,
                    FramePoint(min),
                    FramePoint([
                        min[0] + f64::from(output.width),
                        min[1] + f64::from(output.height),
                    ]),
                    [output.width, output.height],
                )?;
                let plan = Plan::new(Request {
                    output: frame,
                    stages: &self.pipeline,
                    sampling: Sampling::Originals {
                        reconstruction: SourceSupport([0; 2]),
                    },
                })?;
                let SourceRead::Originals(read) = plan.source_read() else {
                    unreachable!()
                };
                Ok(OriginalUnit {
                    plan,
                    output,
                    offset: [output.x - read.x, output.y - read.y],
                })
            })())
        })
    }
}
