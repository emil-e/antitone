//! Bounded image inputs for the CPU donor/healing algorithms, independent of pixel evaluation.

use super::*;

#[cfg(test)]
mod tests;

pub(crate) trait BaseImage {
    /// Return row-major unrepaired AP1. Unneeded cells are zero and need not have valid AP1;
    /// this keeps defects inside the repair mask out of the target's input requirements.
    fn read(
        &mut self,
        region: SourceRegion,
        needed: Option<&[bool]>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<Ap1>>, Error>;
}

impl<F> BaseImage for F
where
    F: FnMut([u32; 2]) -> Result<Ap1, Error>,
{
    fn read(
        &mut self,
        region: SourceRegion,
        needed: Option<&[bool]>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<Ap1>>, Error> {
        let cells = region_cells(region)?;
        if needed.is_some_and(|mask| mask.len() != cells) {
            return Err(Error::InvalidParameter(
                "repair input selection differs from its region",
            ));
        }
        let mut pixels = vec![[0.; 3]; cells];

        for y in 0..region.height {
            for x in 0..region.width {
                if cancelled() {
                    return Ok(None);
                }
                let index = (u64::from(y) * u64::from(region.width) + u64::from(x)) as usize;
                if needed.is_none_or(|mask| mask[index]) {
                    pixels[index] = self([region.x + x, region.y + y])?;
                }
            }
        }

        Ok(Some(pixels))
    }
}

pub(super) fn read(
    image: &mut impl BaseImage,
    region: SourceRegion,
    needed: Option<&[bool]>,
    execution: &mut Execution<'_>,
) -> Result<Option<Vec<Ap1>>, Error> {
    if (execution.cancelled)() {
        return Ok(None);
    }
    let cells = region_cells(region)?;
    if needed.is_some_and(|mask| mask.len() != cells) {
        return Err(Error::InvalidParameter(
            "repair input selection differs from its region",
        ));
    }

    let pixels = image.read(region, needed, execution.cancelled)?;
    if pixels.as_ref().is_some_and(|pixels| pixels.len() != cells) {
        return Err(Error::InvalidParameter(
            "repair input pixels differ from their region",
        ));
    }

    Ok(pixels)
}

fn region_cells(region: SourceRegion) -> Result<usize, Error> {
    region.validate()?;
    usize::try_from(u64::from(region.width) * u64::from(region.height))
        .map_err(|_| Error::InvalidParameter("repair input region is too large"))
}
