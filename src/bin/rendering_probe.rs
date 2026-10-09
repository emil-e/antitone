use antitone::{
    color::BRADFORD,
    profiles,
    rendering::{CpuRenderer, OutputTarget, ocio_version},
};
use glam::{DMat3, DVec3};
use std::{error::Error, fs, path::Path};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn grid(path: &Path) -> Result<Vec<[f32; 4]>> {
    let bytes = fs::read(path)?;
    let (pixels, remainder) = bytes.as_chunks::<16>();
    if !remainder.is_empty() {
        return Err("truncated RGBA fixture".into());
    }
    Ok(pixels
        .iter()
        .map(|pixel| {
            std::array::from_fn(|c| f32::from_le_bytes(pixel[c * 4..c * 4 + 4].try_into().unwrap()))
        })
        .collect())
}

fn write_grid(path: &Path, pixels: &[[f32; 4]]) -> Result<()> {
    let bytes: Vec<_> = pixels
        .iter()
        .flat_map(|pixel| pixel.iter().flat_map(|c| c.to_le_bytes()))
        .collect();
    fs::write(path, bytes)?;
    Ok(())
}

fn matrix(target: OutputTarget) -> DMat3 {
    let p = target.parameters();
    let xyz = |x: f64, y: f64| DVec3::new(x / y, 1.0, (1.0 - x - y) / y);
    let primaries = DMat3::from_cols(xyz(p[1], p[2]), xyz(p[3], p[4]), xyz(p[5], p[6]));
    primaries * DMat3::from_diagonal(primaries.inverse() * xyz(p[7], p[8]))
}

fn gamut_check() -> Result<()> {
    let inputs = [DVec3::new(0.4, 0.4, 0.2), DVec3::splat(0.4)];
    let mut collapsed = false;
    println!(
        "OCIO {}: standalone AP1 yellow/neutral check",
        ocio_version()
    );
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let output = CpuRenderer::new(target)?.render(&inputs)?;
        let to_srgb = target.linear_to_srgb();
        for (input, native) in inputs.into_iter().zip(&output) {
            let common = to_srgb * native.as_dvec3();
            println!(
                "target={target:?} AP1={input:?} native_linear={native:?} linear_sRGB={common:?}"
            );
        }
        let yellow = to_srgb * output[0].as_dvec3();
        collapsed |= yellow.max_element() - yellow.min_element() < 0.01;
        let neutral = to_srgb * output[1].as_dvec3();
        if neutral.max_element() - neutral.min_element() > 2e-5 {
            return Err("neutral rendering is not neutral in the common display basis".into());
        }
    }
    if collapsed {
        return Err(
            "ordinary AP1 yellow collapsed to gray; SDR target gamut validation fails".into(),
        );
    }
    Ok(())
}

fn main() -> Result<()> {
    if std::env::args().nth(1).as_deref() == Some("--gamut-check") {
        return gamut_check();
    }
    let directory = Path::new(".cache/probes");
    let inputs = grid(&directory.join("aces-inputs-ap1.f32"))?;
    let working: Vec<_> = inputs
        .iter()
        .map(|p| DVec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2])))
        .collect();
    println!(
        "OCIO CPU bridge version={} working_space=AP1",
        ocio_version()
    );
    for (name, target) in [
        ("srgb", OutputTarget::Srgb),
        ("display-p3", OutputTarget::DisplayP3),
    ] {
        let reference = grid(&directory.join(format!("{name}-ap1-reference.f32")))?;
        if inputs.len() != reference.len() {
            return Err("CPU fixture length mismatch".into());
        }
        let output = CpuRenderer::new(target)?.render(&working)?;
        let mut maximum = 0.0f32;
        for (actual, expected) in output.iter().zip(reference) {
            for (channel, value) in actual.to_array().into_iter().enumerate() {
                let error = (value - expected[channel]).abs();
                maximum = maximum.max(error);
                if error > 2e-5 + 2e-4 * expected[channel].abs() {
                    return Err("CPU packed/scalar rendering mismatch".into());
                }
            }
        }
        target.encode_u16(&output)?;
        let profile = profiles::icc(target);
        fs::write(directory.join(format!("antitone-{name}-v1.icc")), profile)?;
        println!(
            "target={target:?} pixels={} bulk_vs_scalar_max_error={maximum:.8e} ICC_bytes={}",
            inputs.len(),
            profile.len()
        );
        let p = target.parameters();
        let srgb = OutputTarget::Srgb.parameters();
        let white = |p: [f64; 9]| DVec3::new(p[7] / p[8], 1.0, (1.0 - p[7] - p[8]) / p[8]);
        let adaptation = BRADFORD.inverse()
            * DMat3::from_diagonal((BRADFORD * white(srgb)) / (BRADFORD * white(p)))
            * BRADFORD;
        let to_srgb = matrix(OutputTarget::Srgb).inverse() * adaptation * matrix(target);
        let mut encoded = Vec::new();
        let mut linear_srgb = Vec::new();
        for r in [
            0.0,
            1.0 / 65535.0,
            0.0001,
            1.0 / 2048.0,
            1.0 / 512.0,
            0.18,
            0.5,
            1.0,
        ] {
            for g in [0.0, 0.0001, 1.0 / 512.0, 0.18, 1.0] {
                for b in [0.0, 0.0001, 1.0 / 512.0, 0.18, 1.0] {
                    let linear = DVec3::new(r, g, b);
                    let e = linear.to_array().map(|v| target.encode(v).unwrap() as f32);
                    encoded.push([e[0], e[1], e[2], 1.0]);
                    let s = (to_srgb * linear).as_vec3();
                    linear_srgb.push([s.x, s.y, s.z, 1.0]);
                }
            }
        }
        write_grid(
            &directory.join(format!("{name}-profile-encoded.f32")),
            &encoded,
        )?;
        write_grid(
            &directory.join(format!("{name}-profile-linear-srgb.f32")),
            &linear_srgb,
        )?;
    }
    Ok(())
}
