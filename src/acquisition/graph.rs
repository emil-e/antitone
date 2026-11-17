//! Acquisition envelopes follow the reads of ordinary composed operations.
use super::*;

#[cfg(all(test, feature = "gpu"))]
mod tests;

#[cfg(feature = "gpu")]
pub(super) fn frame_input(
    output: FrameRaster,
    compose: impl FnOnce(crate::gpu::evaluation::Value) -> Result<crate::gpu::evaluation::Value, Error>,
    sampling: Sampling,
) -> Result<(FrameRaster, [u32; 2]), Error> {
    use crate::gpu::evaluation::{CellRegion, Domain, Grid, Meaning, borrowed_input};
    let requested = CellRegion::new([0, 0], output.output).validate()?;
    let input = borrowed_input(Domain {
        bounds: matches!(sampling, Sampling::CameraCells { .. }).then_some(requested),
        grid: Grid::from_frame(output),
        meaning: Meaning::Neutral,
    });
    let required = dependency_bounds(&input, compose, requested)?;
    let low = [required.x, required.y];
    let size = [required.width, required.height];
    let offset = low.map(i32::unsigned_abs);
    Ok((output.processing_region(low.map(f64::from), size)?, offset))
}

#[cfg(feature = "gpu")]
pub(super) fn original_input(
    source: SourceRegion,
    requested: SourceRegion,
    compose: impl FnOnce(crate::gpu::evaluation::Value) -> Result<crate::gpu::evaluation::Value, Error>,
) -> Result<SourceRegion, Error> {
    use crate::gpu::evaluation::{Domain, Grid, Meaning, borrowed_input};
    let input = borrowed_input(Domain {
        bounds: Some(source.try_into()?),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Neutral,
    });
    dependency_bounds(&input, compose, requested.try_into()?)?.source()
}

#[cfg(feature = "gpu")]
fn dependency_bounds(
    input: &crate::gpu::evaluation::Value,
    compose: impl FnOnce(crate::gpu::evaluation::Value) -> Result<crate::gpu::evaluation::Value, Error>,
    requested: crate::gpu::evaluation::CellRegion,
) -> Result<crate::gpu::evaluation::CellRegion, Error> {
    use crate::gpu::evaluation::{Budget, CellRegion};
    let processed = compose(input.clone())?;
    // Retain output cells in the contiguous layout even for translated-only reads.
    let mut low = [requested.x, requested.y];
    let mut high = [requested.right(), requested.bottom()];
    for read in processed.regions_for(
        input,
        requested,
        Budget {
            tile: [512, 128],
            scratch_bytes: u64::MAX,
        },
    )? {
        if !input.domain().contains_region(read) {
            return Err(Error::InvalidParameter(
                "acquisition read exceeds the real input boundary",
            ));
        }
        low[0] = low[0].min(read.x);
        low[1] = low[1].min(read.y);
        high[0] = high[0].max(read.right());
        high[1] = high[1].max(read.bottom());
    }
    let extent = |axis| {
        u32::try_from(i64::from(high[axis]) - i64::from(low[axis]))
            .map_err(|_| Error::InvalidParameter("view dependency exceeds cell coordinates"))
    };
    CellRegion::new(low, [extent(0)?, extent(1)?]).validate()
}

impl Plan {
    /// Derive acquisition geometry without acquiring pixels. The graph receives a neutral-AP1
    /// input on the requested processing grid and declares the original or frame cells it reads.
    pub fn for_graph(
        request: Request,
        compose: impl FnOnce(
            crate::gpu::evaluation::Value,
        ) -> Result<crate::gpu::evaluation::Value, Error>,
    ) -> Result<Self, Error> {
        let mut plan = Self::new(request)?;
        match plan.input {
            ProcessingRaster::Originals(region) => {
                let region = original_input(plan.output.composition.area(), region, compose)?;
                plan.input = ProcessingRaster::Originals(region);
                plan.read = SourceRead::Originals(region);
                plan.valid_output = ValidOutput::Reconstruction {
                    affine: plan.output.affine_in(region)?,
                };
            }
            ProcessingRaster::Frame(_) => {
                let (frame, offset) = frame_input(plan.output, compose, plan.sampling)?;
                plan.input = ProcessingRaster::Frame(frame);
                if matches!(plan.sampling, Sampling::NeutralFootprints) {
                    plan.read = SourceRead::Originals(frame.processing_bounds()?);
                }
                plan.valid_output = ValidOutput::Extraction {
                    size: plan.output.output,
                    offset,
                };
            }
        }
        Ok(plan)
    }
}
