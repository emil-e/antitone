//! Statistical analysis consumes ordered forward-model batches, not a particular pixel evaluator.

use super::*;
use std::ops::Range;

#[cfg(test)]
mod tests;

pub(crate) trait ForwardModel {
    fn read(
        &mut self,
        negative: NegativeInput,
        points: &[DVec3],
        cancelled: &mut dyn FnMut() -> bool,
        consume: &mut dyn FnMut(usize, DVec3) -> Result<(), Error>,
    ) -> Result<bool, Error>;
}

pub(super) struct ScalarReference;

impl ForwardModel for ScalarReference {
    fn read(
        &mut self,
        negative: NegativeInput,
        points: &[DVec3],
        cancelled: &mut dyn FnMut() -> bool,
        consume: &mut dyn FnMut(usize, DVec3) -> Result<(), Error>,
    ) -> Result<bool, Error> {
        let inversion = Inversion::new(negative)?;
        for (index, &point) in points.iter().enumerate() {
            if cancelled() {
                return Ok(false);
            }
            consume(index, inversion.apply(point)?.rgb)?;
        }
        Ok(true)
    }
}

pub(super) struct SampleBatch {
    pub points: Vec<DVec3>,
    pub groups: Vec<Range<usize>>,
}

impl SampleBatch {
    pub fn new<'a>(groups: impl IntoIterator<Item = &'a [DVec3]>) -> Self {
        let mut points = Vec::new();
        let mut ranges = Vec::new();
        for group in groups {
            let start = points.len();
            points.extend_from_slice(group);
            ranges.push(start..points.len());
        }
        Self {
            points,
            groups: ranges,
        }
    }

    pub fn read(
        &self,
        model: &mut impl ForwardModel,
        negative: NegativeInput,
        cancelled: &mut dyn FnMut() -> bool,
        mut consume: impl FnMut(usize, usize, DVec3) -> Result<(), Error>,
    ) -> Result<bool, Error> {
        let mut next = 0;
        let mut group = 0;
        let completed = model.read(negative, &self.points, cancelled, &mut |index, positive| {
            if index != next || index >= self.points.len() {
                return Err(Error::InvalidParameter(
                    "analysis model changed sample order",
                ));
            }
            if !positive.is_finite() {
                return Err(Error::NumericalOverflow);
            }
            while index >= self.groups[group].end {
                group += 1;
            }
            consume(group, index - self.groups[group].start, positive)?;
            next += 1;
            Ok(())
        })?;
        if completed && next != self.points.len() {
            return Err(Error::InvalidParameter("analysis model omitted samples"));
        }
        Ok(completed)
    }
}
