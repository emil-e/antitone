//! Original native response, used only to regenerate the derived source volume.
use super::{curve::Curve, require};
use crate::Error;
use glam::DVec3;
pub(super) const SIZE: usize = 32;
pub(super) const NODES: usize = SIZE.pow(3);
pub(super) struct Response {
    shapers: [Curve; 3],
    nodes: Vec<[f32; 3]>,
}
impl Response {
    pub(super) fn new(shapers: [Curve; 3], nodes: Vec<[f32; 3]>) -> Result<Self, Error> {
        for curve in &shapers {
            curve.validate()?;
        }
        require(
            nodes.len() == NODES
                && nodes
                    .iter()
                    .flatten()
                    .all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
            "invalid native Frontier colour volume",
        )?;
        Ok(Self { shapers, nodes })
    }
    pub(super) fn sample_codes(&self, grb: [f64; 3]) -> DVec3 {
        let shaped: [f64; 3] = std::array::from_fn(|c| self.shapers[c].sample(grb[c]) / 1023.);
        let q = DVec3::new(shaped[1], shaped[0], shaped[2]) * (33. / 32.) * (SIZE - 1) as f64;
        let cell = q
            .to_array()
            .map(|p| (p.floor().max(0.) as usize).min(SIZE - 2));
        let f = q - DVec3::from_array(cell.map(|v| v as f64));
        let mut out = DVec3::ZERO;
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    let weight = (if r == 0 { 1. - f.x } else { f.x })
                        * (if g == 0 { 1. - f.y } else { f.y })
                        * (if b == 0 { 1. - f.z } else { f.z });
                    out += DVec3::from_array(
                        self.nodes[((cell[2] + b) * SIZE + cell[1] + g) * SIZE + cell[0] + r]
                            .map(f64::from),
                    ) * weight;
                }
            }
        }
        out
    }
}
