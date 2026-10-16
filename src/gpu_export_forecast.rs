//! Optional future-use hints. Exact reconstruction remains the authority for required originals.
use super::*;

const MAX_BLOCKS: u32 = 8192;
const MAX_INDEX_BYTES: usize = 1024 * 1024;
const MAX_BLOCK_ORIGINS: usize = 9;
const ORDER: export::BlockOrder = export::BlockOrder::Spatial1x5;

type Origin = [u32; 2];

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Use {
    origin: Origin,
    block: u32,
}

pub(super) struct Forecast {
    uses: Vec<Use>,
    blocks: u32,
}

impl Forecast {
    pub fn new(composition: Composition) -> Option<Self> {
        Self::with_budget(composition, MAX_INDEX_BYTES)
    }

    fn with_budget(composition: Composition, budget: usize) -> Option<Self> {
        let [width, height] = composition.size();
        let columns = width.div_ceil(export::BLOCK[0]);
        let rows = height.div_ceil(export::BLOCK[1]);
        let blocks = columns.checked_mul(rows)?;
        if composition.is_exact() || blocks == 0 || blocks > MAX_BLOCKS {
            return None;
        }

        let raster = FrameRaster::full(composition).ok()?;
        let size = ImageSize { width, height };
        let budget = budget.min(MAX_INDEX_BYTES);
        let mut count = 0usize;

        // Count with stack hints before allocating the bounded flat index once.
        for (origin, extent) in export::ordered_blocks(size, ORDER) {
            count = count.checked_add(hints(raster, origin, extent)?.len)?;
            if count.checked_mul(std::mem::size_of::<Use>())? > budget {
                return None;
            }
        }

        let mut uses = Vec::new();
        uses.try_reserve_exact(count).ok()?;
        if uses.capacity().checked_mul(std::mem::size_of::<Use>())? > budget {
            return None;
        }

        for (block, (origin, extent)) in export::ordered_blocks(size, ORDER).enumerate() {
            let hints = hints(raster, origin, extent)?;
            for &origin in &hints.origins[..hints.len] {
                if uses.len() == count {
                    return None;
                }
                uses.push(Use {
                    origin,
                    block: block as u32,
                });
            }
        }
        if uses.len() != count {
            return None;
        }

        uses.sort_unstable();
        Some(Self { uses, blocks })
    }

    pub fn order(&self) -> export::BlockOrder {
        ORDER
    }

    #[cfg(test)]
    pub fn payload_bytes(&self) -> usize {
        self.uses.capacity() * std::mem::size_of::<Use>()
    }

    fn next(&self, origin: Origin, block: u32) -> u32 {
        let at = self
            .uses
            .partition_point(|entry| *entry <= Use { origin, block });
        self.uses
            .get(at)
            .filter(|entry| entry.origin == origin)
            .map_or(u32::MAX, |entry| entry.block)
    }
}

pub(super) struct Cursor {
    forecast: Forecast,
    block: Option<u32>,
}

impl Cursor {
    pub fn new(forecast: Forecast) -> Self {
        Self {
            forecast,
            block: None,
        }
    }

    pub fn order(&self) -> export::BlockOrder {
        self.forecast.order()
    }

    pub fn advance(&mut self) -> bool {
        let next = match self.block {
            Some(block) => block.checked_add(1),
            None => Some(0),
        };
        let Some(next) = next.filter(|&block| block < self.forecast.blocks) else {
            return false;
        };
        self.block = Some(next);
        true
    }

    /// None bypasses retention, never the load or return of the current original.
    pub fn victim(&self, incoming: Origin, keys: impl Iterator<Item = Origin>) -> Option<usize> {
        let Some(block) = self.block else {
            return Some(0);
        };
        let (victim, farthest) = keys
            .enumerate()
            .map(|(index, key)| (index, self.forecast.next(key, block)))
            .max_by_key(|&(index, next)| (next, std::cmp::Reverse(index)))?;
        (self.forecast.next(incoming, block) < farthest).then_some(victim)
    }
}

struct Hints {
    origins: [Origin; MAX_BLOCK_ORIGINS],
    len: usize,
}

fn hints(raster: FrameRaster, origin: Origin, extent: Origin) -> Option<Hints> {
    let [x, y] = origin.map(f64::from);
    let [width, height] = extent.map(f64::from);
    let bounds = Coverage::new(
        [
            [x, y],
            [x + width, y],
            [x + width, y + height],
            [x, y + height],
        ]
        .map(|point| raster.source_point(point)),
    )
    .ok()?
    .bounds(raster.composition.area())
    .ok()?;
    let area = raster.composition.area();
    let tile = tile_size(area);
    let first = [(bounds.x - area.x) / tile[0], (bounds.y - area.y) / tile[1]];
    let last = [
        (bounds.x + bounds.width - 1 - area.x) / tile[0],
        (bounds.y + bounds.height - 1 - area.y) / tile[1],
    ];
    let columns = last[0].checked_sub(first[0])?.checked_add(1)?;
    let rows = last[1].checked_sub(first[1])?.checked_add(1)?;
    if columns.checked_mul(rows)? > MAX_BLOCK_ORIGINS as u32 {
        return None;
    }
    let mut hints = Hints {
        origins: [[0; 2]; MAX_BLOCK_ORIGINS],
        len: 0,
    };
    for y in first[1]..=last[1] {
        for x in first[0]..=last[0] {
            // Assignment bins are source-relative; a partial final bin loads an overlapping tile.
            let key = [
                (area.x + x * tile[0]).min(area.x + area.width - tile[0]),
                (area.y + y * tile[1]).min(area.y + area.height - tile[1]),
            ];
            if !hints.origins[..hints.len].contains(&key) {
                hints.origins[hints.len] = key;
                hints.len += 1;
            }
        }
    }
    Some(hints)
}

#[cfg(test)]
#[path = "gpu_export_forecast_tests.rs"]
mod tests;
