//! Compatibility of acquired samples, independent of photographic stage parameters.
#[cfg(test)]
mod tests;

use super::*;

impl Plan {
    pub fn for_output(&self, output: FrameRaster) -> Result<Self, Error> {
        Self::new(Request {
            output,
            stages: &self.pipeline,
            sampling: self.sampling,
        })
    }

    pub fn for_pipeline(&self, stages: &PixelPipeline) -> Result<Self, Error> {
        Self::new(Request {
            output: self.output,
            stages,
            sampling: self.sampling,
        })
    }

    /// Neutral samples depend on layout/representation, not the downstream parameter values.
    pub fn same_acquisition_layout(&self, other: &Self) -> bool {
        self.output == other.output
            && self.sampling == other.sampling
            && self.input == other.input
            && self.read == other.read
            && self.valid_output == other.valid_output
    }

    /// Both plans request the same output grid; acquired neighbours may exceed demand.
    pub fn can_supply(&self, demand: &Self) -> bool {
        if self.output != demand.output || self.sampling != demand.sampling {
            return false;
        }
        match (
            self.input,
            demand.input,
            self.valid_output,
            demand.valid_output,
        ) {
            (ProcessingRaster::Originals(have), ProcessingRaster::Originals(need), _, _) => {
                have.contains_region(need)
            }
            (
                ProcessingRaster::Frame(_),
                ProcessingRaster::Frame(_),
                ValidOutput::Extraction { offset: have, .. },
                ValidOutput::Extraction { offset: need, .. },
            ) => {
                // Symmetric same-grid margins define integral offsets in a shared output grid.
                // Comparing expanded floating pitches would turn harmless roundoff into misses.
                (0..2).all(|i| have[i] >= need[i])
            }
            _ => false,
        }
    }

    /// Execute current intent against a compatible acquired raster without relabelling its layout.
    pub fn using_input(&self, available: &Self) -> Result<Self, Error> {
        if !available.can_supply(self) {
            return Err(Error::InvalidParameter(
                "acquired raster lacks required support or representation",
            ));
        }
        let mut plan = self.clone();
        plan.input = available.input;
        plan.read = available.read;
        plan.valid_output = available.valid_output;
        Ok(plan)
    }

    pub fn processing_frame(&self) -> Result<FrameRaster, Error> {
        match self.input {
            ProcessingRaster::Frame(frame) => Ok(frame),
            _ => Err(Error::InvalidParameter(
                "processing plan is not on a frame grid",
            )),
        }
    }
}
