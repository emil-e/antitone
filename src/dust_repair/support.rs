use super::*;
use std::sync::Arc;

// Bound both marked samples and the raster used for grouping/reconstruction.
// Search reads separate target/donor crops rather than expanding this raster.
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

pub fn offset_target(
    support: &SupportMask,
    active: SourceRegion,
    offset: [i32; 2],
) -> Result<SourceRegion, Error> {
    let target = donor_target(support, active)?;
    let source_x = i64::from(target.x) + i64::from(offset[0]);
    let source_y = i64::from(target.y) + i64::from(offset[1]);
    let source_right = source_x + i64::from(target.width);
    let source_bottom = source_y + i64::from(target.height);
    if source_x < i64::from(active.x)
        || source_y < i64::from(active.y)
        || source_right > i64::from(active.x + active.width)
        || source_bottom > i64::from(active.y + active.height)
    {
        return Err(Error::InvalidParameter("donor outside source"));
    }
    Ok(SourceRegion {
        x: source_x as u32,
        y: source_y as u32,
        width: target.width,
        height: target.height,
    })
}

pub(super) struct RequiredFootprint {
    origin: [i32; 2],
    size: [u32; 2],
    points: Vec<[i32; 2]>,
}

impl RequiredFootprint {
    pub(super) fn new(support: &SupportMask) -> Self {
        let width = support.size[0] as usize;
        let height = support.size[1] as usize;
        let canvas_width = width + 4;
        let canvas_height = height + 4;
        let mut bits = vec![false; canvas_width * canvas_height];
        for [x, y] in support.selected_points() {
            let local_x = (x - support.origin[0]) as usize + 2;
            let local_y = (y - support.origin[1]) as usize + 2;
            bits[local_y * canvas_width + local_x] = true;
        }
        let required = dilate(&bits, canvas_width, canvas_height, 2);
        let mut points = Vec::new();
        for y in 0..canvas_height {
            for x in 0..canvas_width {
                if required[y * canvas_width + x] {
                    points.push([
                        support.origin[0] as i32 + x as i32 - 2,
                        support.origin[1] as i32 + y as i32 - 2,
                    ]);
                }
            }
        }
        Self {
            origin: [support.origin[0] as i32 - 2, support.origin[1] as i32 - 2],
            size: [canvas_width as u32, canvas_height as u32],
            points,
        }
    }

    pub(super) fn clear(&self, support: &SupportMask, offset: [i32; 2]) -> bool {
        let x0 = self.origin[0] + offset[0];
        let y0 = self.origin[1] + offset[1];
        let support_x0 = support.origin[0] as i32;
        let support_y0 = support.origin[1] as i32;
        let support_x1 = support_x0 + support.size[0] as i32;
        let support_y1 = support_y0 + support.size[1] as i32;
        if x0 + self.size[0] as i32 <= support_x0
            || y0 + self.size[1] as i32 <= support_y0
            || x0 >= support_x1
            || y0 >= support_y1
        {
            return true;
        }
        for [x, y] in &self.points {
            let point = [x + offset[0], y + offset[1]];
            if point[0] >= support_x0
                && point[0] < support_x1
                && point[1] >= support_y0
                && point[1] < support_y1
                && support.contains([point[0] as u32, point[1] as u32])
            {
                return false;
            }
        }
        true
    }
}

pub fn validate_offset(
    offset: [i32; 2],
    support: &SupportMask,
    active: SourceRegion,
) -> Result<(), Error> {
    offset_target(support, active, offset)?;
    let footprint = RequiredFootprint::new(support);
    if !footprint.clear(support, offset) {
        return Err(Error::InvalidParameter("donor intersects support"));
    }
    Ok(())
}

pub(crate) fn visit_offsets(
    support: &SupportMask,
    active: SourceRegion,
    stopped: &dyn Fn() -> bool,
    mut visit: impl FnMut([i32; 2]) -> bool,
) {
    let Ok(target) = donor_target(support, active) else {
        return;
    };
    let dx_min = i64::from(active.x) - i64::from(target.x);
    let dx_max = i64::from(active.x + active.width) - i64::from(target.x + target.width);
    let dy_min = i64::from(active.y) - i64::from(target.y);
    let dy_max = i64::from(active.y + active.height) - i64::from(target.y + target.height);
    if dx_min > dx_max || dy_min > dy_max {
        return;
    }
    let footprint = RequiredFootprint::new(support);
    let max_radius = dx_min
        .unsigned_abs()
        .max(dx_max.unsigned_abs())
        .max(dy_min.unsigned_abs())
        .max(dy_max.unsigned_abs());
    for radius in 0..=max_radius {
        if stopped() {
            return;
        }
        for (dx, dy) in ring(radius as i64) {
            if stopped() {
                return;
            }
            if dx < dx_min || dx > dx_max || dy < dy_min || dy > dy_max {
                continue;
            }
            let offset = [dx as i32, dy as i32];
            if footprint.clear(support, offset) && !visit(offset) {
                return;
            }
        }
    }
}

/// Find a clean placement without walking every translation's footprint.
/// Disjoint bounding boxes usually supply an immediate witness. For cramped
/// sources, row-run differences describe exactly the forbidden translations.
/// Successor links mark each forbidden x at most once per translated row.
pub fn initial_offset(support: &SupportMask, active: SourceRegion) -> Option<[i32; 2]> {
    let target = donor_target(support, active).ok()?;
    let minimum = [
        active.x as i32 - target.x as i32,
        active.y as i32 - target.y as i32,
    ];
    let maximum = [
        (active.x + active.width - target.x - target.width) as i32,
        (active.y + active.height - target.y - target.height) as i32,
    ];
    let footprint = RequiredFootprint::new(support);
    let mut witnesses = [
        [-(support.size[0] as i32 + 2), 0],
        [support.size[0] as i32 + 2, 0],
        [0, -(support.size[1] as i32 + 2)],
        [0, support.size[1] as i32 + 2],
    ];
    witnesses.sort_by_key(|[x, y]| i64::from(*x).pow(2) + i64::from(*y).pow(2));
    for offset in witnesses {
        if (0..2).all(|axis| offset[axis] >= minimum[axis] && offset[axis] <= maximum[axis])
            && footprint.clear(support, offset)
        {
            return Some(offset);
        }
    }

    let rows = |points: Vec<[i32; 2]>, origin: [i32; 2], height: u32| {
        let mut rows = vec![Vec::<[i32; 2]>::new(); height as usize];
        for [x, y] in points {
            let row = &mut rows[(y - origin[1]) as usize];
            if let Some(run) = row.last_mut()
                && run[1] == x
            {
                run[1] += 1;
            } else {
                row.push([x, x + 1]);
            }
        }
        rows
    };
    let support_rows = rows(
        support
            .selected_points()
            .map(|p| p.map(|v| v as i32))
            .collect(),
        support.origin.map(|v| v as i32),
        support.size[1],
    );
    let required_rows = rows(footprint.points, footprint.origin, footprint.size[1]);
    let width = (maximum[0] - minimum[0] + 1) as usize;
    let radius = minimum[1].unsigned_abs().max(maximum[1].unsigned_abs());
    for dy in std::iter::once(0).chain((1..=radius).flat_map(|r| [r as i32, -(r as i32)])) {
        if dy < minimum[1] || dy > maximum[1] {
            continue;
        }
        let mut next = (0..=width).collect::<Vec<_>>();
        'rows: for (y, required) in required_rows.iter().enumerate() {
            let sy = footprint.origin[1] + y as i32 + dy - support.origin[1] as i32;
            let Some(row) = usize::try_from(sy).ok().and_then(|y| support_rows.get(y)) else {
                continue;
            };
            for &[r0, r1] in required {
                for &[s0, s1] in row {
                    let left = (s0 - r1 + 1).max(minimum[0]);
                    let right = (s1 - r0 - 1).min(maximum[0]);
                    if left > right {
                        continue;
                    }
                    let end = (right - minimum[0]) as usize;
                    let mut x = successor(&mut next, (left - minimum[0]) as usize);
                    while x <= end {
                        next[x] = successor(&mut next, x + 1);
                        x = next[x];
                    }
                    if successor(&mut next, 0) == width {
                        break 'rows;
                    }
                }
            }
        }
        if let Some(x) = (0..width)
            .filter(|x| next[*x] == *x)
            .min_by_key(|x| (minimum[0] + *x as i32).unsigned_abs())
        {
            return Some([minimum[0] + x as i32, dy]);
        }
    }
    None
}

fn successor(next: &mut [usize], mut index: usize) -> usize {
    while next[index] != index {
        next[index] = next[next[index]];
        index = next[index];
    }
    index
}

pub fn nearest_offset(support: &SupportMask, active: SourceRegion) -> Option<[i32; 2]> {
    let mut nearest = None;
    visit_offsets(support, active, &|| false, |offset| {
        nearest = Some(offset);
        false
    });
    nearest
}

/// Chebyshev-ring offsets, deterministic order. Corners are emitted once.
fn ring(radius: i64) -> impl Iterator<Item = (i64, i64)> {
    let mut positions = Vec::new();
    if radius == 0 {
        positions.push((0, 0));
    } else {
        for x in -radius..=radius {
            positions.push((x, -radius));
            positions.push((x, radius));
        }
        for y in (-radius + 1)..radius {
            positions.push((-radius, y));
            positions.push((radius, y));
        }
    }
    positions.into_iter()
}
