//! Scalar original-grid measurements from explicitly supplied camera light and correction.

use crate::{
    Error,
    coverage::Coverage,
    flat_field::{FlatField, sample_camera_light},
    raw::Rect,
    scan::DevelopedSource,
};
use glam::{DMat3, DVec3};

/// Mean corrected camera light in a rectangular calibration patch.
pub fn reference(
    source: &DevelopedSource,
    correction: Option<&FlatField>,
    region: Rect,
) -> Result<DVec3, Error> {
    source.validate_area(region)?;
    let mean = if correction.is_some() {
        let mut sum = DVec3::ZERO;
        for y in region.y..region.y + region.height {
            for x in region.x..region.x + region.width {
                sum += sample_camera_light(source, correction, [x, y])?;
            }
        }
        sum / (f64::from(region.width) * f64::from(region.height))
    } else {
        source
            .camera_image()?
            .sample_mean(region, DMat3::IDENTITY)?
    };
    require_positive(mean)
}

/// Mean corrected camera light weighted by the patch's coverage of each original pixel.
pub fn reference_coverage(
    source: &DevelopedSource,
    correction: Option<&FlatField>,
    coverage: Coverage,
) -> Result<DVec3, Error> {
    if let Some(region) = coverage.exact_rectangle(source.area()) {
        return reference(source, correction, region);
    }
    require_positive(super::weighted_mean_iter(
        coverage.pixels(source.area())?.map(|(point, weight)| {
            sample_camera_light(source, correction, point).map(|pixel| (pixel, weight))
        }),
    )?)
}

fn require_positive(mean: DVec3) -> Result<DVec3, Error> {
    if !mean.is_finite() || mean.min_element() <= 0. {
        return Err(Error::UnusableSample(
            "reference mean must be finite and positive in every channel",
        ));
    }
    Ok(mean)
}

#[cfg(all(test, feature = "editor"))]
pub(crate) fn white_balance(
    source: &DevelopedSource,
    correction: Option<&FlatField>,
    coverage: Coverage,
    upstream: &crate::adjustment_layers::AdjustmentLayerPipeline,
    layer: Option<crate::adjustment_layers::AdjustmentLayer>,
) -> Result<crate::color::Balance, Error> {
    let pixels = coverage
        .pixels(source.area())?
        .map(|(point @ [x, y], area)| {
            let camera = sample_camera_light(source, correction, point)?;
            let result = upstream.apply(camera, point)?;
            let weight = match layer {
                Some(layer) => layer.weight(crate::geometry::SourcePoint([
                    f64::from(x) + 0.5,
                    f64::from(y) + 0.5,
                ]))?,
                None => 1.,
            };
            let exposure = layer.map_or(0., |layer| layer.photo.exposure) * weight;
            Ok((
                result.rgb * crate::processing::exposure_multiplier(exposure)?,
                area,
                weight,
            ))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    match layer {
        Some(_) => super::adjustment_layer_white_balance(&pixels),
        None => crate::color::neutral_wb(super::weighted_mean_iter(
            pixels.into_iter().map(|(rgb, area, _)| Ok((rgb, area))),
        )?),
    }
}
