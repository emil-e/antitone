//! Shared final-cube sampling; providers own the material response and provenance.
use super::{Lut3D, authoring, failure, require};
use crate::{Error, photographic};
use glam::DVec3;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(super) fn bake(
    size: u32,
    cancel: &AtomicBool,
    sample: &(impl Fn(DVec3) -> Result<DVec3, Error> + Sync),
) -> Result<Arc<Lut3D>, Error> {
    require(
        (2..=super::MAX_GRID_SIZE).contains(&size),
        "unsupported material LUT grid",
    )?;
    let n = size as usize;
    let lo = authoring::DOMAIN_MIN;
    let step = (authoring::domain_max() - lo) / f64::from(size - 1);
    let workers = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(4)
        .min(n);
    let chunks = std::thread::scope(|scope| {
        let handles = (0..workers)
            .map(|worker| {
                scope.spawn(move || {
                    let start = worker * n / workers;
                    let end = (worker + 1) * n / workers;
                    let mut values = Vec::with_capacity((end - start) * n * n);
                    for b in start..end {
                        for g in 0..n {
                            if cancel.load(Ordering::Relaxed) {
                                return Err(failure("look preparation cancelled"));
                            }
                            for r in 0..n {
                                let rgb = DVec3::new(r as f64, g as f64, b as f64)
                                    .map(|v| photographic::decode(lo + v * step));
                                values.push(sample(rgb)?.to_array().map(|v| v as f32));
                            }
                        }
                    }
                    Ok(values)
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .map_err(|_| failure("material bake worker panicked"))?
            })
            .collect::<Result<Vec<_>, Error>>()
    })?;
    Ok(Arc::new(Lut3D::new(
        size,
        [lo as f32; 3],
        [authoring::domain_max() as f32; 3],
        chunks.into_iter().flatten().collect(),
    )?))
}
