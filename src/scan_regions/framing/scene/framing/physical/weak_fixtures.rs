//! Declared picture coverage and paired IID/correlated noise, not detector-generated truth.
use super::super::tests::fixture;
use super::tests::geometry;
use super::*;

struct Noise(u64);
impl Noise {
    fn uniform(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
        ((self.0 >> 32) as f64 + 0.5) / (u32::MAX as f64 + 1.)
    }
    fn normal(&mut self) -> f64 {
        (-2. * self.uniform().ln()).sqrt() * (std::f64::consts::TAU * self.uniform()).cos()
    }
}

pub(super) fn surface(
    seed: u64,
    density: f64,
    correlated: bool,
    blank: bool,
) -> (Capture, Evidence, Scene, PictureGeometry) {
    render(seed, density, correlated, blank, None, 2.)
}

pub(super) fn soft_surface(seed: u64, blur: f64) -> (Capture, Evidence, Scene, PictureGeometry) {
    render(seed, 0.04, false, false, None, blur)
}

pub(super) fn patch_surface(
    seed: u64,
    density: f64,
    correlated: bool,
    patch: usize,
) -> (Capture, Evidence, Scene, PictureGeometry) {
    render(seed, density, correlated, false, Some(patch), 2.)
}

fn render(
    seed: u64,
    density: f64,
    correlated: bool,
    blank: bool,
    patch: Option<usize>,
    blur: f64,
) -> (Capture, Evidence, Scene, PictureGeometry) {
    let (mut capture, mut evidence, scene) = fixture([128, 96], [16., 112., 12., 84.], None, false);
    let phase = (seed % 4) as f64 * 0.25;
    let frame = geometry(capture.size, [16. + phase, 112. + phase, 12., 84.]);
    let mut rng = Noise(seed);
    // Independent coarse-grid fields create correlation without drawing a border.
    let nodes: Vec<[f64; 3]> = (0..17 * 13)
        .map(|_| std::array::from_fn(|_| rng.normal()))
        .collect();
    for y in 0..96 {
        for x in 0..128 {
            let px = x as f64 + 0.5;
            let py = y as f64 + 0.5;
            let distance = (px - 16. - phase)
                .min(112. + phase - px)
                .min(py - 12.)
                .min(84. - py);
            let coverage = (distance / blur + 0.5).clamp(0., 1.);
            let band = ((py - 12.).max(0.) / 9.).floor() as usize % 8;
            let weight = if let Some(patch) = patch {
                if band == patch { 6. } else { 0. }
            } else {
                [0.5, 1.5, 0., 1., 0.75, 1.25, 0., 1.][band]
            };
            let amplitude = weight * density;
            let edge_weight = ((px - 96. - phase) / 8.).clamp(0., 1.);
            let attenuation = if blank {
                0.
            } else {
                coverage * (0.8 * (1. - edge_weight) + amplitude * edge_weight)
            };
            let gx = x / 8;
            let gy = y / 8;
            let tx = (x % 8) as f64 / 8.;
            let ty = (y % 8) as f64 / 8.;
            capture.rgb[y * 128 + x] = std::array::from_fn(|ch| {
                let field = (1. - ty)
                    * ((1. - tx) * nodes[gy * 17 + gx][ch] + tx * nodes[gy * 17 + gx + 1][ch])
                    + ty * ((1. - tx) * nodes[(gy + 1) * 17 + gx][ch]
                        + tx * nodes[(gy + 1) * 17 + gx + 1][ch]);
                let noise = if correlated {
                    0.01 * rng.normal() + 0.04 * field
                } else {
                    0.03 * rng.normal()
                };
                (20000. * (noise - attenuation).exp2())
                    .round()
                    .clamp(2., 65534.) as u16
            });
        }
    }
    // Discovery cannot be credited with a supplied native cue for the weak side.
    evidence.lines.get_mut("right").unwrap().clear();
    if blank {
        for lines in evidence.lines.values_mut() {
            lines.clear();
        }
    }
    (capture, evidence, scene, frame)
}
