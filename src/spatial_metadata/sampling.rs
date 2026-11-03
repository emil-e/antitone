use crate::scan_regions::{Areas, Material, PictureGeometry, Quad};
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

/// Finished automatic statistical windows, distinct from physical framing and authored edits.
/// Material classification belongs to SourceRegions and is attached when constructing Areas.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SamplingAreas {
    opening: Option<Quad>,
    picture: Option<Quad>,
    opening_observed: [bool; 4],
    opening_shared: bool,
    opening_exception: bool,
    picture_geometry: Option<PictureGeometry>,
    picture_constraints: Vec<[f64; 3]>,
    picture_observed: [bool; 4],
    picture_weak: [bool; 4],
}

impl SamplingAreas {
    pub(crate) fn new(areas: &Areas) -> Self {
        Self {
            opening: areas.opening,
            picture: areas.picture,
            opening_observed: areas.opening_observed,
            opening_shared: areas.opening_shared,
            opening_exception: areas.opening_exception,
            picture_geometry: areas.picture_geometry.clone(),
            picture_constraints: areas.picture_constraints.clone(),
            picture_observed: areas.picture_observed,
            picture_weak: areas.picture_weak,
        }
    }

    pub(super) fn validate(
        &self,
        active: crate::geometry::SourceRegion,
    ) -> Result<(), crate::Error> {
        self.areas(None).validate(active)
    }

    pub(super) fn areas(&self, material: Option<Material>) -> Areas {
        Areas {
            opening: self.opening,
            picture: self.picture,
            material,
            opening_observed: self.opening_observed,
            opening_shared: self.opening_shared,
            opening_exception: self.opening_exception,
            picture_geometry: self.picture_geometry.clone(),
            picture_constraints: self.picture_constraints.clone(),
            picture_observed: self.picture_observed,
            picture_weak: self.picture_weak,
            opening_edited: false,
            picture_edited: false,
        }
    }
}
