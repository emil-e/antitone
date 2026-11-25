//! Pointwise native-camera response, shared by gamma picking and statistical model evaluation.

use super::*;
use crate::photographic::graph::PhotographGraph;
use crate::{
    photographic::stages::display_rendering as drt,
    photographic::stages::interpretation::InputTransform,
    processing::{Inversion, NegativeInput},
    rendering::OutputTarget,
};

#[cfg(test)]
mod tests;

pub(crate) fn normalized(camera: Value, negative: NegativeInput) -> Result<Value, Error> {
    let program = InputTransform::new(negative, glam::DMat3::IDENTITY)?;
    PhotographGraph::native_positive(camera, program)
}

/// Provisional positive for semantic sampling: fixed sRGB DRT, no creative output stages.
pub(crate) fn classifier_image(
    size: [u32; 2],
    camera: &[glam::DVec3],
    negative: NegativeInput,
    post: glam::DMat3,
    drt: &drt::Target,
) -> Result<Value, Error> {
    if drt.output_target() != OutputTarget::Srgb {
        return Err(Error::InvalidParameter(
            "classification requires the sRGB DRT",
        ));
    }
    if size.contains(&0) || u64::from(size[0]) * u64::from(size[1]) != camera.len() as u64 {
        return Err(Error::InvalidParameter(
            "classifier raster does not match camera samples",
        ));
    }
    // Keep the provisional fit's f64 inversion and working-space mapping up to GPU upload.
    let inversion = Inversion::new(negative)?;
    let points = camera
        .iter()
        .map(|&point| inversion.apply(point).map(|positive| post * positive.rgb))
        .collect::<Result<_, _>>()?;
    drt.apply(source(Arc::new(ProvisionalPositive { size, points })))
}

struct ProvisionalPositive {
    size: [u32; 2],
    points: Vec<glam::DVec3>,
}

impl Source for ProvisionalPositive {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some(CellRegion::new([0, 0], self.size)),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        }
    }

    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        if !self.domain().contains_region(region) {
            return Err(Error::InvalidParameter("read exceeds classifier raster"));
        }
        let region = region.source()?;
        let stride = self.size[0] as usize;
        let mut pixels = Vec::with_capacity(region.width as usize * region.height as usize);
        for y in region.y..region.bottom() {
            let first = y as usize * stride + region.x as usize;
            for point in &self.points[first..first + region.width as usize] {
                let rgb = point.as_vec3();
                if !rgb.is_finite() {
                    return Err(Error::InvalidParameter(
                        "analysis corrected camera samples exceed the f32 domain",
                    ));
                }
                pixels.push([rgb.x, rgb.y, rgb.z, 0.]);
            }
        }
        Ok(pixels)
    }
}
