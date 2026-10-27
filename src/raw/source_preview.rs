use super::{DMat3, DVec3, Metadata};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SourceWhiteBalance {
    #[default]
    Daylight,
    AsShot,
}

/// Display-only AP1 transforms, derived while source headers are already open.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SourcePreview {
    daylight: Option<DMat3>,
    as_shot: Option<DMat3>,
}

impl SourcePreview {
    pub(crate) fn transform(self, white: SourceWhiteBalance) -> DMat3 {
        match white {
            SourceWhiteBalance::Daylight => self.daylight,
            SourceWhiteBalance::AsShot => self.as_shot.or(self.daylight),
        }
        .unwrap_or(DMat3::IDENTITY)
    }
}

impl Metadata {
    pub(crate) fn source_preview(&self, fixed_wb: [f64; 4]) -> SourcePreview {
        let Ok(basis) = self.camera_to_ap1() else {
            return SourcePreview::default();
        };
        #[cfg(feature = "dng-sdk")]
        if let Some(header) = self.sdk {
            // Both SDK matrices map the developed buffer to PCS. Their ratio is
            // the viewing adjustment in the cached source's existing AP1 basis.
            let pcs_to_ap1 = basis * header.camera_to_xyz_d50.inverse();
            let transform = |view| valid_transform(pcs_to_ap1 * view * basis.inverse());
            return SourcePreview {
                daylight: transform(header.daylight_to_xyz_d50),
                as_shot: transform(header.as_shot_to_xyz_d50),
            };
        }
        let transform = |desired| native_transform(basis, fixed_wb, desired);
        SourcePreview {
            daylight: transform(self.daylight_mul),
            as_shot: transform(self.as_shot_wb()),
        }
    }
}

fn native_transform(basis: DMat3, fixed: [f64; 4], desired: [f64; 4]) -> Option<DMat3> {
    let normalized = |wb: [f64; 4]| {
        // Development uses f32 gains. Green-relative ratios pair with the source
        // preview's separate compensation for LibRaw's max-gain normalization.
        let [r, g, b, _] = wb.map(|v| f64::from(v as f32));
        let rgb = DVec3::new(r, g, b);
        (rgb.is_finite() && rgb.min_element() > 0.).then(|| rgb / g)
    };
    let ratio = normalized(desired)? / normalized(fixed)?;
    let limit = crate::processing::MAX_EXPOSURE.exp2();
    if !ratio.is_finite() || ratio.min_element() < 1. / limit || ratio.max_element() > limit {
        return None;
    }
    valid_transform(basis * DMat3::from_diagonal(ratio) * basis.inverse())
}

fn valid_transform(matrix: DMat3) -> Option<DMat3> {
    (matrix.is_finite()
        && matrix.determinant().abs() > 1e-12
        && matrix
            .to_cols_array()
            .iter()
            .all(|v| (*v as f32).is_finite()))
    .then_some(matrix)
}

#[cfg(test)]
mod tests;
