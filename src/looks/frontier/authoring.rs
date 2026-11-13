//! Offline authoring from caller-supplied original parameter files; never used by rendering.
use super::{Variant, density, failure, require};
mod curve;
mod native;
use crate::{
    Error,
    rendering::{CpuRenderer, OutputTarget, reference_parameters},
};
use curve::Curve;
use glam::DVec3;
use std::{fs, io::Write, path::Path};

const GRID: usize = 129;
const WIDTH: f64 = 1. / 1024.;

/// Read original software data without executing it and write only derived AP1 responses.
/// `destination` must not exist, so regeneration cannot replace accepted assets in place.
pub fn generate(
    colour_data: &Path,
    negative_setup: &Path,
    destination: &Path,
) -> Result<(), Error> {
    let colour = fs::read(colour_data).map_err(failure)?;
    let setup = fs::read(negative_setup).map_err(failure)?;
    require(
        blake3::hash(&colour).to_hex().as_str()
            == "3264270f481e588ddbd815bed2d09a00d082209264adac49ede577671cace913"
            && blake3::hash(&setup).to_hex().as_str()
                == "9428360c4a523288d0050447e0d9f05a94421c86881807f6a306bd22cc83a84b",
        "unsupported Frontier source revision",
    )?;
    require(
        colour.len() >= 0x84000 + 3 * native::NODES && setup.len() >= 0x2e,
        "incomplete Frontier parameter files",
    )?;
    require(
        i16::from_le_bytes(setup[0x2a..0x2c].try_into().unwrap()) == 450
            && i16::from_le_bytes(setup[0x2c..0x2e].try_into().unwrap()) == 512,
        "unsupported Frontier source-mode origins",
    )?;
    require(
        !destination.exists(),
        "choose a fresh Frontier candidate directory",
    )?;
    let inverse = CpuRenderer::new_inverse_reference(reference_parameters(OutputTarget::Srgb))?;
    let anchors = [
        [410.43065207185845, 422.6037663707722, 424.35454914309963],
        [475.5513132216693, 488.6933467623055, 485.5754511774941],
    ];
    let mut responses = Vec::new();
    for (index, variant) in Variant::all().enumerate() {
        let mut curves = Vec::new();
        for (channel, &anchor) in anchors[index].iter().enumerate() {
            let offset = index * 0x1800 + channel * 0x800;
            let ordinates: Vec<_> = colour[offset..offset + 2048]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|p| u16::from_le_bytes(*p))
                .collect();
            curves.push(Curve::from_runs(&ordinates, anchor)?);
        }
        let offset = [0x3c000, 0x84000][index];
        let nodes = (0..native::NODES)
            .map(|i| {
                std::array::from_fn(|c| {
                    (f64::from(colour[offset + c * native::NODES + i]) / 255.) as f32
                })
            })
            .collect();
        let source = native::Response::new(curves.try_into().unwrap(), nodes)?;
        let linear: Vec<_> = (0..GRID.pow(3))
            .map(|i| {
                let code = DVec3::new(
                    (i % GRID) as f64,
                    ((i / GRID) % GRID) as f64,
                    (i / GRID / GRID) as f64,
                ) * (1023. / (GRID - 1) as f64);
                source
                    .sample_codes([code.y, code.x, code.z])
                    .map(|v| decode(landing(v)))
            })
            .collect();
        let carrier = inverse.apply_unbounded(&linear)?;
        let dense: Vec<_> = carrier.iter().map(|v| v.as_dvec3()).collect();
        let smooth = gaussian(dense, GRID);
        let nodes = (0..density::NODES)
            .map(|i| {
                let r = 2 * (i % density::SIZE);
                let g = 2 * ((i / density::SIZE) % density::SIZE);
                let b = 2 * (i / density::SIZE / density::SIZE);
                smooth[r + GRID * (g + GRID * b)]
                    .to_array()
                    .map(|v| v as f32)
            })
            .collect();
        responses.push((variant, density::Response::new(nodes)?));
    }
    fs::create_dir(destination).map_err(failure)?;
    let mut hashes = serde_json::Map::new();
    for (variant, response) in responses {
        let name = match variant {
            Variant::A => "density-a.f32.z",
            Variant::B => "density-b.f32.z",
        };
        let bytes = response.bytes();
        hashes.insert(
            name.into(),
            blake3::hash(&bytes).to_hex().to_string().into(),
        );
        let file = fs::File::create(destination.join(name)).map_err(failure)?;
        let mut compressor = flate2::write::ZlibEncoder::new(file, flate2::Compression::best());
        compressor.write_all(&bytes).map_err(failure)?;
        compressor.finish().map_err(failure)?;
    }
    let metadata = serde_json::json!({
        "contract": "frontier-density65-ap1-d60-v1",
        "source_blake3": {"ColorData.dat": blake3::hash(&colour).to_hex().to_string(), "negative_setup": blake3::hash(&setup).to_hex().to_string()},
        "raw_payload_blake3": hashes,
        "input": "formed native0..1023 RGB codes; uniform65 grid; first axis fastest",
        "output": "AP1/D60 authored rendering carrier; ordinary selected-output ACES follows",
        "authoring": {"grid": GRID, "sigma": 0.5, "radius": 2, "boundary": "constant edge extension", "final_grid": density::SIZE, "space": "Gaussian AP1 response volume in code coordinates"},
        "shaper_anchors_GRB": anchors,
        "reference": {"OCIO": crate::rendering::ocio_version(), "parameters": reference_parameters(OutputTarget::Srgb), "encoded_continuation_width": WIDTH},
    });
    fs::write(
        destination.join("generation.json"),
        serde_json::to_vec_pretty(&metadata).map_err(failure)?,
    )
    .map_err(failure)
}
fn landing(v: f64) -> f64 {
    if v < WIDTH {
        WIDTH * WIDTH / (2. * WIDTH - v)
    } else if v > 1. - WIDTH {
        1. - WIDTH * WIDTH / (v - 1. + 2. * WIDTH)
    } else {
        v
    }
}
fn decode(v: f64) -> f64 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
fn gaussian(mut nodes: Vec<DVec3>, grid: usize) -> Vec<DVec3> {
    let mut weights = [-2., -1., 0., 1., 2.].map(|v: f64| (-0.5 * (v / 0.5).powi(2)).exp());
    let total: f64 = weights.iter().sum();
    for w in &mut weights {
        *w /= total;
    }
    let mut output = vec![DVec3::ZERO; nodes.len()];
    // First array axis is blue because red is fastest in the serialized volume.
    for stride in [grid * grid, grid, 1] {
        for (i, out) in output.iter_mut().enumerate() {
            let coordinate = (i / stride) % grid;
            let mut value = DVec3::ZERO;
            for (j, &weight) in weights.iter().enumerate() {
                let next =
                    (coordinate as isize + j as isize - 2).clamp(0, grid as isize - 1) as usize;
                let index = i - coordinate * stride + next * stride;
                value += nodes[index] * weight;
            }
            *out = value;
        }
        std::mem::swap(&mut nodes, &mut output);
    }
    nodes
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn smoothing_preserves_constants_and_affine_interior_without_crossing_colour_channels() {
        let n = 9;
        let c = DVec3::new(-0.2, 0.7, 8.);
        let values = gaussian(vec![c; n * n * n], n);
        assert!(values.iter().all(|v| (*v - c).abs().max_element() < 2e-14));
        let affine =
            |i: usize| DVec3::new((i % n) as f64, ((i / n) % n) as f64, (i / n / n) as f64);
        let values = gaussian((0..n * n * n).map(affine).collect(), n);
        for b in 2..n - 2 {
            for g in 2..n - 2 {
                for r in 2..n - 2 {
                    let i = r + n * (g + n * b);
                    assert!((values[i] - affine(i)).abs().max_element() < 2e-14);
                }
            }
        }
        assert!(values[0].min_element() > 0.);
        assert!(values[n * n * n - 1].max_element() < (n - 1) as f64);
    }
}
