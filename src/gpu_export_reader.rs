//! Job-owned tile reads: reuse immutable values before loading new origins.
use super::{
    Error,
    cache::{SharedTile, TileCache},
};

type Pixel = [f64; 4];

pub(super) trait TileReader {
    type Pixels: AsRef<[Pixel]>;
    fn read(&mut self, origin: [u32; 2]) -> Result<Self::Pixels, Error>;
    fn prepare(&mut self, _origins: impl Iterator<Item = [u32; 2]>) -> Result<(), Error> {
        Ok(())
    }
    fn finish(&mut self) -> Result<(), Error> {
        Ok(())
    }
    fn order(&self) -> Option<super::export::BlockOrder> {
        None
    }
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
    Shared(SharedTile),
}

impl AsRef<[Pixel]> for Pixels {
    fn as_ref(&self) -> &[Pixel] {
        match self {
            Self::Owned(p) => p,
            Self::Shared(p) => p.as_ref(),
        }
    }
}

pub(super) trait TileLoader {
    fn load(&mut self, origin: [u32; 2]) -> Result<Vec<Pixel>, Error>;
    fn prepare(&mut self, _misses: impl Iterator<Item = [u32; 2]>) -> Result<(), Error> {
        Ok(())
    }
    fn finish(&mut self) -> Result<(), Error> {
        Ok(())
    }
}

impl<F: FnMut([u32; 2]) -> Result<Vec<Pixel>, Error>> TileLoader for F {
    fn load(&mut self, origin: [u32; 2]) -> Result<Vec<Pixel>, Error> {
        self(origin)
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

    pub fn forecasted(load: L, check: C, forecast: super::forecast::Forecast) -> Self {
        Self {
            load,
            check,
            cache: Some(TileCache::forecasted(forecast)),
            requests: 0,
        }
    }

    pub fn into_loader(self) -> L {
        self.load
    }

    pub fn peak_bytes(&self) -> usize {
        self.cache.as_ref().map_or(0, TileCache::peak_bytes)
    }
}

impl<L, C> TileReader for JobReader<L, C>
where
    L: TileLoader,
    C: FnMut() -> Result<(), Error>,
{
    type Pixels = Pixels;

    fn read(&mut self, origin: [u32; 2]) -> Result<Pixels, Error> {
        (self.check)()?;
        self.requests += 1;
        let pixels = match &mut self.cache {
            Some(cache) => Pixels::Shared(cache.read(origin, |o| self.load.load(o))?),
            None => Pixels::Owned(self.load.load(origin)?),
        };
        (self.check)()?;
        Ok(pixels)
    }

    fn finish(&mut self) -> Result<(), Error> {
        self.load.finish()
    }

    fn prepare(&mut self, origins: impl Iterator<Item = [u32; 2]>) -> Result<(), Error> {
        if let Some(cache) = &mut self.cache {
            cache.begin_block();
        }
        let cache = &self.cache;
        self.load
            .prepare(origins.filter(|o| !cache.as_ref().is_some_and(|c| c.contains(*o))))
    }

    fn order(&self) -> Option<super::export::BlockOrder> {
        self.cache.as_ref().and_then(TileCache::order)
    }

    fn cached(&self, origin: [u32; 2]) -> bool {
        self.cache
            .as_ref()
            .is_some_and(|cache| cache.contains(origin))
    }
}
