use super::*;
use std::sync::Arc;

// The padded 512-square work limit admits thin supports without increasing worst-case work.
pub(crate) const MAX_MASK_WORK_PIXELS: u64 =
    (512 + 2 * (CONTEXT_PADDING as u64 + AUTOMATIC_PADDING as u64)).pow(2);
pub(crate) const MAX_MASK_PIXELS: usize = 65_536;

#[cfg(test)]
mod tests;

pub fn mask_within_budget(size: [u32; 2], marked: usize) -> bool {
    let padding = 2 * (u64::from(CONTEXT_PADDING) + u64::from(AUTOMATIC_PADDING));
    marked <= MAX_MASK_PIXELS
        && (u64::from(size[0]) + padding)
            .checked_mul(u64::from(size[1]) + padding)
            .is_some_and(|cells| cells <= MAX_MASK_WORK_PIXELS)
}

/// Exact binary support, retained as maximal row-major `[start, length]` runs.
/// Immutable runs are shared by recipes and history; decoding is reserved for
/// bounded solver/grouping work, not project ownership.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "project", serde(deny_unknown_fields))]
pub struct SupportMask {
    pub(super) origin: [u32; 2],
    pub(super) size: [u32; 2],
    runs: Arc<[[u32; 2]]>,
}

impl SupportMask {
    pub fn new(origin: [u32; 2], size: [u32; 2], bits: Vec<bool>) -> Result<Self, Error> {
        let count = Self::checked_count(origin, size)?;
        if bits.len() != count as usize {
            return Err(Error::InvalidParameter("invalid repair support bits"));
        }

        let mut runs = Vec::new();
        let mut index = 0;
        while index < bits.len() {
            if !bits[index] {
                index += 1;
                continue;
            }
            let start = index;
            while index < bits.len() && bits[index] {
                index += 1;
            }
            runs.push([start as u32, (index - start) as u32]);
        }

        let support = Self {
            origin,
            size,
            runs: runs.into(),
        };
        support.validate()?;
        Ok(support)
    }

    fn checked_count(origin: [u32; 2], size: [u32; 2]) -> Result<u32, Error> {
        origin[0]
            .checked_add(size[0])
            .and_then(|_| origin[1].checked_add(size[1]))
            .ok_or(Error::InvalidParameter(
                "repair support coordinates overflow",
            ))?;
        size[0]
            .checked_mul(size[1])
            .filter(|count| *count > 0)
            .ok_or(Error::InvalidParameter("invalid repair support dimensions"))
    }

    pub(crate) fn validate(&self) -> Result<(), Error> {
        let count = Self::checked_count(self.origin, self.size)?;

        let mut previous_end = None;
        let mut marked = 0usize;
        for &[start, length] in self.runs.iter() {
            let end = start.checked_add(length).filter(|end| *end <= count);
            if length == 0 || end.is_none() || previous_end.is_some_and(|end| start <= end) {
                return Err(Error::InvalidParameter("invalid repair support runs"));
            }
            marked = marked
                .checked_add(length as usize)
                .ok_or(Error::InvalidParameter("repair support is too large"))?;
            previous_end = end;
        }

        if marked == 0 || !mask_within_budget(self.size, marked) {
            return Err(Error::InvalidParameter(
                "dust repair mask is empty or too large",
            ));
        }
        Ok(())
    }

    pub fn origin(&self) -> [u32; 2] {
        self.origin
    }

    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    /// Logical retained payload, conservatively charging shared runs to each owner.
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + std::mem::size_of_val(self.runs.as_ref())
    }

    pub fn marked_pixels(&self) -> usize {
        self.runs.iter().map(|run| run[1] as usize).sum()
    }

    pub fn bits(&self) -> Vec<bool> {
        let mut bits = vec![false; (self.size[0] * self.size[1]) as usize];
        for &[start, length] in self.runs.iter() {
            bits[start as usize..(start + length) as usize].fill(true);
        }
        bits
    }

    pub(super) fn selected_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.runs
            .iter()
            .flat_map(|&[start, length]| (start..start + length).map(|i| i as usize))
    }

    pub fn selected_points(&self) -> impl Iterator<Item = [u32; 2]> + '_ {
        self.selected_indices().map(|index| {
            [
                self.origin[0] + (index % self.size[0] as usize) as u32,
                self.origin[1] + (index / self.size[0] as usize) as u32,
            ]
        })
    }

    /// Exact support groups in the healer's donor order, for donor geometry presentation.
    pub fn grouped_supports(&self) -> Vec<Self> {
        let width = self.size[0] as usize;
        grouped_components(&self.bits(), width, self.size[1] as usize)
            .into_iter()
            .map(|group| {
                let min_x = group.iter().map(|i| i % width).min().unwrap();
                let max_x = group.iter().map(|i| i % width).max().unwrap();
                let min_y = group.iter().map(|i| i / width).min().unwrap();
                let max_y = group.iter().map(|i| i / width).max().unwrap();
                let size = [(max_x - min_x + 1) as u32, (max_y - min_y + 1) as u32];
                let mut bits = vec![false; (size[0] * size[1]) as usize];
                for i in group {
                    bits[(i / width - min_y) * size[0] as usize + i % width - min_x] = true;
                }
                Self::new(
                    [self.origin[0] + min_x as u32, self.origin[1] + min_y as u32],
                    size,
                    bits,
                )
                .expect("validated nonempty donor group")
            })
            .collect()
    }

    pub fn contains(&self, [x, y]: [u32; 2]) -> bool {
        if x < self.origin[0]
            || y < self.origin[1]
            || x >= self.origin[0] + self.size[0]
            || y >= self.origin[1] + self.size[1]
        {
            return false;
        }

        let index = (y - self.origin[1]) * self.size[0] + x - self.origin[0];
        let run = self.runs.partition_point(|[start, _]| *start <= index);
        run > 0 && index < self.runs[run - 1][0] + self.runs[run - 1][1]
    }

    pub(crate) fn intersects(&self, other: &Self) -> bool {
        if self.origin[0] >= other.origin[0] + other.size[0]
            || other.origin[0] >= self.origin[0] + self.size[0]
            || self.origin[1] >= other.origin[1] + other.size[1]
            || other.origin[1] >= self.origin[1] + self.size[1]
        {
            return false;
        }
        self.selected_points().any(|point| other.contains(point))
    }
}

pub fn donor_target(support: &SupportMask, active: SourceRegion) -> Result<SourceRegion, Error> {
    let mut points = support.selected_points();
    let first = points
        .next()
        .ok_or(Error::InvalidParameter("empty repair mask"))?;
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (first[0], first[0], first[1], first[1]);
    for [x, y] in points {
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    Ok(SourceRegion {
        x: min_x.saturating_sub(CONTEXT_PADDING).max(active.x),
        y: min_y.saturating_sub(CONTEXT_PADDING).max(active.y),
        width: max_x
            .saturating_add(CONTEXT_PADDING + 1)
            .min(active.x + active.width)
            - min_x.saturating_sub(CONTEXT_PADDING).max(active.x),
        height: max_y
            .saturating_add(CONTEXT_PADDING + 1)
            .min(active.y + active.height)
            - min_y.saturating_sub(CONTEXT_PADDING).max(active.y),
    })
}

pub fn validate_donor(
    choice: DonorChoice,
    support: &SupportMask,
    active: SourceRegion,
) -> Result<(), Error> {
    let DonorChoice::Offset([dx, dy]) = choice else {
        return Ok(());
    };

    let target = donor_target(support, active)?;
    let source_x = i64::from(target.x) + i64::from(dx);
    let source_y = i64::from(target.y) + i64::from(dy);
    let source_right = source_x + i64::from(target.width);
    let source_bottom = source_y + i64::from(target.height);
    if source_x < i64::from(active.x)
        || source_y < i64::from(active.y)
        || source_right > i64::from(active.x + active.width)
        || source_bottom > i64::from(active.y + active.height)
    {
        return Err(Error::InvalidParameter("manual donor outside source"));
    }

    if support.selected_points().any(|[x, y]| {
        i64::from(x) >= source_x
            && i64::from(x) < source_right
            && i64::from(y) >= source_y
            && i64::from(y) < source_bottom
    }) {
        return Err(Error::InvalidParameter(
            "manual donor intersects required pixels",
        ));
    }
    Ok(())
}
