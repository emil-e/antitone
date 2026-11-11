//! Acquisition envelopes follow the reads of ordinary composed operations.
use super::*;

#[cfg(all(test, feature = "gpu"))]
mod tests;

#[cfg(feature = "gpu")]
pub(super) fn frame_input(
    output: FrameRaster,
    pipeline: &PixelPipeline,
    sampling: Sampling,
) -> Result<(FrameRaster, [u32; 2]), Error> {
    use crate::gpu::evaluation::{CellRegion, Domain, Grid, Meaning, borrowed_input};
    let requested = CellRegion::new([0, 0], output.output).validate()?;
    let input = borrowed_input(Domain {
        bounds: matches!(sampling, Sampling::CameraCells { .. }).then_some(requested),
        grid: Grid::from_frame(output),
        meaning: Meaning::Graded,
    });
    let required = dependency_bounds(&input, pipeline, requested)?;
    let low = [required.x, required.y];
    let size = [required.width, required.height];
    let offset = low.map(i32::unsigned_abs);
    Ok((output.processing_region(low.map(f64::from), size)?, offset))
}

#[cfg(feature = "gpu")]
pub(super) fn original_input(
    source: SourceRegion,
    requested: SourceRegion,
    pipeline: &PixelPipeline,
) -> Result<SourceRegion, Error> {
    use crate::gpu::evaluation::{Domain, Grid, Meaning, borrowed_input};
    let input = borrowed_input(Domain {
        bounds: Some(source.try_into()?),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Graded,
    });
    dependency_bounds(&input, pipeline, requested.try_into()?)?.source()
}

#[cfg(feature = "gpu")]
fn dependency_bounds(
    input: &crate::gpu::evaluation::Value,
    pipeline: &PixelPipeline,
    requested: crate::gpu::evaluation::CellRegion,
) -> Result<crate::gpu::evaluation::CellRegion, Error> {
    use crate::gpu::evaluation::{Budget, CellRegion};
    let processed = pipeline.compose(input.clone())?;
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

#[cfg(not(feature = "gpu"))]
pub(super) fn original_input(
    _: SourceRegion,
    requested: SourceRegion,
    pipeline: &PixelPipeline,
) -> Result<SourceRegion, Error> {
    require_empty(pipeline)?;
    Ok(requested)
}

#[cfg(not(feature = "gpu"))]
fn require_empty(pipeline: &PixelPipeline) -> Result<(), Error> {
    if !pipeline.is_empty() {
        return Err(Error::InvalidParameter(
            "effect-node acquisition planning requires the gpu feature",
        ));
    }
    Ok(())
}

#[cfg(not(feature = "gpu"))]
pub(super) fn frame_input(
    output: FrameRaster,
    pipeline: &PixelPipeline,
    _: Sampling,
) -> Result<(FrameRaster, [u32; 2]), Error> {
    require_empty(pipeline)?;
    Ok((output, [0; 2]))
}
