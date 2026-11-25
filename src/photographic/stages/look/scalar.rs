use super::{decode, encode};
use crate::{
    Error,
    looks::{Lut3D, require},
};
use glam::DVec3;

impl Lut3D {
    /// Double-precision tetrahedral reference with component-wise input clipping.
    pub fn sample_encoded(&self, input: DVec3) -> Result<DVec3, Error> {
        let input = input.to_array();
        let (min, max) = self.domain();
        let size = self.size();
        let mut cell = [0; 3];
        let mut fraction = [0.; 3];
        for c in 0..3 {
            let lo = f64::from(min[c]);
            let hi = f64::from(max[c]);
            require(input[c].is_finite(), "nonfinite ACEScct look input")?;
            let position = (input[c].clamp(lo, hi) - lo) / (hi - lo) * f64::from(size - 1);
            cell[c] = (position as u32).min(size - 2);
            fraction[c] = position - f64::from(cell[c]);
        }
        let at = |p: [u32; 3]| {
            DVec3::from_array(
                self.nodes()[((p[2] * size + p[1]) * size + p[0]) as usize].map(f64::from),
            )
        };
        let mut order = [0, 1, 2];
        order.sort_by(|&a, &b| fraction[b].total_cmp(&fraction[a]));
        let [a, b, c] = order.map(|i| fraction[i]);
        let mut out = at(cell) * (1. - a);
        cell[order[0]] += 1;
        out += at(cell) * (a - b);
        cell[order[1]] += 1;
        out += at(cell) * (b - c);
        cell[order[2]] += 1;
        Ok(out + at(cell) * c)
    }

    pub fn apply(&self, linear_ap1: DVec3) -> Result<DVec3, Error> {
        require(linear_ap1.is_finite(), "nonfinite scene-linear look input")?;
        let (min, max) = self.domain();
        // A finite negative AP1 value can overflow ACEScct's linear branch.
        // Clip the encoded result before admission; nonfinite source values still reject.
        let encoded = linear_ap1.map(encode).clamp(
            DVec3::from_array(min.map(f64::from)),
            DVec3::from_array(max.map(f64::from)),
        );
        let output = self.sample_encoded(encoded)?.map(decode);
        require(output.is_finite(), "nonfinite scene-linear look output")?;
        Ok(output)
    }
}
