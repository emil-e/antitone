//! Completed original-tile values owned only by one immutable export job.
use super::{Error, TILE};
use std::{collections::VecDeque, sync::Arc};

// Share the slice owner without copying a completed tile into a second sample allocation.
#[derive(Clone)]
pub(super) struct SharedTile(Arc<Box<[[f64; 4]]>>);
impl AsRef<[[f64; 4]]> for SharedTile {
    fn as_ref(&self) -> &[[f64; 4]] {
        self.0.as_ref().as_ref()
    }
}

// Four entries thrash in some traversal directions; the four-turn work test covers this bound.
const CAPACITY: usize = 8;
pub(super) const MAX_PAYLOAD_BYTES: usize =
    CAPACITY * TILE[0] as usize * TILE[1] as usize * std::mem::size_of::<[f64; 4]>();

// Source, correction, grade and format are immutable in the owning job; origin suffices here.
// Never retain this cache across jobs.
pub(super) struct TileCache {
    entries: VecDeque<([u32; 2], SharedTile)>,
    peak_bytes: usize,
}
impl TileCache {
    pub(super) fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            peak_bytes: 0,
        }
    }

    pub(super) fn contains(&self, origin: [u32; 2]) -> bool {
        self.entries.iter().any(|(key, _)| *key == origin)
    }

    pub(super) fn read(
        &mut self,
        origin: [u32; 2],
        mut load: impl FnMut([u32; 2]) -> Result<Vec<[f64; 4]>, Error>,
    ) -> Result<SharedTile, Error> {
        if let Some(index) = self.entries.iter().position(|(key, _)| *key == origin) {
            let entry = self.entries.remove(index).unwrap();
            let result = entry.1.clone();
            self.entries.push_back(entry);
            return Ok(result);
        }
        let pixels = load(origin)?;
        if std::mem::size_of_val(pixels.as_slice()) > MAX_PAYLOAD_BYTES / CAPACITY {
            return Err(Error::InvalidParameter(
                "original tile exceeds export cache payload bound",
            ));
        }
        // Boxed slices retain the sample payload, not spare Vec capacity.
        let tile = SharedTile(Arc::new(pixels.into_boxed_slice()));
        if self.entries.len() == CAPACITY {
            self.entries.pop_front();
        }
        self.entries.push_back((origin, tile.clone()));
        self.peak_bytes = self.peak_bytes.max(
            self.entries
                .iter()
                .map(|(_, tile)| std::mem::size_of_val(tile.as_ref()))
                .sum(),
        );
        Ok(tile)
    }

    pub(super) fn peak_bytes(&self) -> usize {
        self.peak_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_reuses_immutable_values_evicts_lru_and_keeps_payload_bounded() {
        let mut cache = TileCache::new();
        let mut loads = 0;
        let mut load = |origin: [u32; 2]| {
            loads += 1;
            Ok(vec![
                [origin[0] as f64, -1., 4., 7.];
                (TILE[0] * TILE[1]) as usize
            ])
        };
        let retained = cache.read([0, 0], &mut load).unwrap();
        for x in 1..CAPACITY as u32 {
            cache.read([x, 0], &mut load).unwrap();
        }
        let hit = cache
            .read([0, 0], |_| panic!("cache hit must not load"))
            .unwrap();
        assert!(Arc::ptr_eq(&retained.0, &hit.0));
        cache.read([CAPACITY as u32, 0], &mut load).unwrap();
        cache
            .read([0, 0], |_| panic!("MRU value must survive"))
            .unwrap();
        cache.read([1, 0], &mut load).unwrap();
        assert_eq!(loads, CAPACITY + 2);
        assert_eq!(cache.entries.len(), CAPACITY);
        assert_eq!(cache.peak_bytes(), MAX_PAYLOAD_BYTES);
        drop(cache);
        assert_eq!(retained.as_ref()[0], [0., -1., 4., 7.]);
        assert_eq!(Arc::strong_count(&retained.0), 2);
        drop(hit);
        assert_eq!(Arc::strong_count(&retained.0), 1);
    }

    #[test]
    fn failed_loads_are_not_cached_and_jobs_never_share_values() {
        let mut cache = TileCache::new();
        assert!(
            cache
                .read([0, 0], |_| Err(Error::Gpu("readback failed".into())))
                .is_err()
        );
        let first = cache.read([0, 0], |_| Ok(vec![[1.; 4]])).unwrap();
        let mut another = TileCache::new();
        let second = another.read([0, 0], |_| Ok(vec![[2.; 4]])).unwrap();
        assert_eq!(first.as_ref(), &[[1.; 4]]);
        assert_eq!(second.as_ref(), &[[2.; 4]]);
        assert!(!Arc::ptr_eq(&first.0, &second.0));
        assert!(
            cache
                .read([1, 0], |_| Ok(vec![
                    [0.; 4];
                    (TILE[0] * TILE[1]) as usize + 1
                ]))
                .is_err()
        );
        assert_eq!(cache.entries.len(), 1);
    }
}
