//! Explicit neutral image values for mathematical fixtures, not runtime repair preparation.
use crate::{
    Error,
    dust_repair::{EvaluatedRepair, SupportMask},
    gpu::evaluation::*,
};
use std::sync::Arc;

pub(crate) type Replacement = (SupportMask, Value);

pub(crate) fn replacements(values: &[Arc<EvaluatedRepair>]) -> Result<Vec<Replacement>, Error> {
    values
        .iter()
        .map(|value| {
            let support = value.support().clone();
            let [x, y] = support.origin();
            let [width, height] = support.size();
            let region = CellRegion::new(
                [
                    i32::try_from(x).map_err(|_| {
                        Error::InvalidParameter("fixture origin exceeds cell coordinates")
                    })?,
                    i32::try_from(y).map_err(|_| {
                        Error::InvalidParameter("fixture origin exceeds cell coordinates")
                    })?,
                ],
                [width, height],
            )
            .validate()?;
            let mut pixels = vec![[0.; 4]; width as usize * height as usize];
            for (point, rgb) in value.pixels() {
                if !rgb.iter().all(|value| value.is_finite()) {
                    return Err(Error::NumericalOverflow);
                }
                pixels[region.index(point.map(|v| v as i32))] = [rgb[0], rgb[1], rgb[2], 0.];
            }
            let image = source(Arc::new(Pixels {
                domain: Domain {
                    bounds: Some(region),
                    grid: Grid::axis_aligned([0.; 2], [1.; 2]),
                    meaning: Meaning::Neutral,
                },
                region,
                pixels,
            }));
            Ok((support, image))
        })
        .collect()
}

pub(crate) fn image(domain: Domain, pixels: Vec<[f32; 4]>) -> Result<Value, Error> {
    let region = domain.finite()?;
    if pixels.len() != region.width as usize * region.height as usize {
        return Err(Error::InvalidParameter(
            "fixture image omits declared values",
        ));
    }
    Ok(source(Arc::new(Pixels {
        domain,
        region,
        pixels,
    })))
}

struct Pixels {
    domain: Domain,
    region: CellRegion,
    pixels: Vec<[f32; 4]>,
}
impl Source for Pixels {
    fn domain(&self) -> Domain {
        self.domain
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(
            self.pixels
                .iter()
                .map(|pixel| pixel.map(f32::to_bits))
                .collect::<Vec<_>>(),
        ))
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        if !self.region.contains_region(region) {
            return Err(Error::InvalidParameter(
                "fixture image read exceeds declared values",
            ));
        }
        let mut values = Vec::with_capacity(region.width as usize * region.height as usize);
        for y in region.y..region.bottom() {
            let start = self.region.index([region.x, y]);
            values.extend_from_slice(&self.pixels[start..start + region.width as usize]);
        }
        Ok(values)
    }
}

pub(crate) fn same(a: &[Replacement], b: &[Replacement]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|((am, ai), (bm, bi))| am == bm && ai.same_definition(bi))
}
