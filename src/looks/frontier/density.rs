//! Smoothed formed-code response, with AP1 output ready for selected-output rendering.
use super::{Variant, failure, require};
use crate::Error;
use glam::DVec3;
use std::{
    io::Read,
    sync::{Arc, OnceLock},
};

pub(crate) const SIZE: usize = 65;
pub(crate) const NODES: usize = SIZE.pow(3);
const BYTES: usize = NODES * 12;

#[derive(Debug)]
pub(super) struct Response {
    nodes: Vec<[f32; 3]>,
}
impl Response {
    pub(super) fn new(nodes: Vec<[f32; 3]>) -> Result<Self, Error> {
        require(
            nodes.len() == NODES && nodes.iter().flatten().all(|v| v.is_finite()),
            "invalid AP1 density response",
        )?;
        Ok(Self { nodes })
    }
    fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        require(
            bytes.len() == BYTES,
            "incorrect density response byte count",
        )?;
        Self::new(
            bytes
                .as_chunks::<12>()
                .0
                .iter()
                .map(|p| {
                    std::array::from_fn(|c| {
                        f32::from_le_bytes(p[c * 4..c * 4 + 4].try_into().unwrap())
                    })
                })
                .collect(),
        )
    }
    #[cfg(feature = "ocio-reference")]
    pub(super) fn bytes(&self) -> Vec<u8> {
        self.nodes
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect()
    }
    pub(crate) fn sample(&self, code: DVec3) -> DVec3 {
        let q = code * ((SIZE - 1) as f64 / 1023.);
        let cell = q
            .to_array()
            .map(|v| (v.floor().max(0.) as usize).min(SIZE - 2));
        let f = q - DVec3::from_array(cell.map(|v| v as f64));
        let mut out = DVec3::ZERO;
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    let weight = (if r == 0 { 1. - f.x } else { f.x })
                        * (if g == 0 { 1. - f.y } else { f.y })
                        * (if b == 0 { 1. - f.z } else { f.z });
                    let index = (cell[0] + r) + SIZE * ((cell[1] + g) + SIZE * (cell[2] + b));
                    out += DVec3::from_array(self.nodes[index].map(f64::from)) * weight;
                }
            }
        }
        out
    }
}
pub(super) fn builtin(variant: Variant) -> Result<Arc<Response>, Error> {
    static A: OnceLock<Result<Arc<Response>, Error>> = OnceLock::new();
    static B: OnceLock<Result<Arc<Response>, Error>> = OnceLock::new();
    let (slot, bytes) = match variant {
        Variant::A => (
            &A,
            include_bytes!("../../../assets/frontier/density-a.f32.z").as_slice(),
        ),
        Variant::B => (
            &B,
            include_bytes!("../../../assets/frontier/density-b.f32.z").as_slice(),
        ),
    };
    slot.get_or_init(|| {
        let mut decoded = Vec::new();
        let mut decoder = flate2::bufread::ZlibDecoder::new(bytes);
        decoder
            .by_ref()
            .take(BYTES as u64 + 1)
            .read_to_end(&mut decoded)
            .map_err(failure)?;
        require(
            decoder.total_in() as usize == bytes.len(),
            "trailing density response data",
        )?;
        Response::from_bytes(&decoded).map(Arc::new)
    })
    .clone()
}
