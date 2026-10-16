//! Job-owned tile reads: reuse immutable values before loading new origins.
use super::{
    Error,
    cache::{SharedTile, TileCache},
};

type Pixel = [f64; 4];
pub(super) trait TileReader {
    type Pixels: AsRef<[Pixel]>;
    fn read(&mut self, origin: [u32; 2]) -> Result<Self::Pixels, Error>;
    fn cached(&self, _origin: [u32; 2]) -> bool {
        false
    }
}

#[cfg(all(test, target_os = "macos"))]
pub(super) struct Callback<F>(pub F);
#[cfg(all(test, target_os = "macos"))]
impl<F, P> TileReader for Callback<F>
where
    F: FnMut([u32; 2]) -> Result<P, Error>,
    P: AsRef<[Pixel]>,
{
    type Pixels = P;
    fn read(&mut self, origin: [u32; 2]) -> Result<P, Error> {
        (self.0)(origin)
    }
}

pub(super) enum Pixels {
    Owned(Vec<Pixel>),
    Cached(SharedTile),
}
impl AsRef<[Pixel]> for Pixels {
    fn as_ref(&self) -> &[Pixel] {
        match self {
            Self::Owned(p) => p,
            Self::Cached(p) => p.as_ref(),
        }
    }
}

pub(super) struct JobReader<L, C> {
    load: L,
    check: C,
    cache: Option<TileCache>,
    pub requests: u64,
}
impl<L, C> JobReader<L, C> {
    pub fn new(load: L, check: C, cached: bool) -> Self {
        Self {
            load,
            check,
            cache: cached.then(TileCache::new),
            requests: 0,
        }
    }
    pub fn peak_bytes(&self) -> usize {
        self.cache.as_ref().map_or(0, TileCache::peak_bytes)
    }
}
impl<L, C> TileReader for JobReader<L, C>
where
    L: FnMut([u32; 2]) -> Result<Vec<Pixel>, Error>,
    C: FnMut() -> Result<(), Error>,
{
    type Pixels = Pixels;
    fn read(&mut self, origin: [u32; 2]) -> Result<Pixels, Error> {
        (self.check)()?;
        self.requests += 1;
        let pixels = match &mut self.cache {
            Some(cache) => Pixels::Cached(cache.read(origin, &mut self.load)?),
            None => Pixels::Owned((self.load)(origin)?),
        };
        (self.check)()?;
        Ok(pixels)
    }
    fn cached(&self, origin: [u32; 2]) -> bool {
        self.cache
            .as_ref()
            .is_some_and(|cache| cache.contains(origin))
    }
}
