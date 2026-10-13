mod operators;
mod simple;

use antitone::{
    processing::{Calibration, PhotoSettings, Pipeline, RollSettings},
    profiles,
    raw::Rect,
    rendering::{CpuRenderer, OutputTarget, ocio_version},
    scan::Scan,
};
use glam::{DVec3, Vec3};
use operators::{Contrast, Saturation, blend, chroma, contrast, saturation, y};
use std::{error::Error, fs, io::Write, path::Path};
use tiff::{
    encoder::{TiffEncoder, colortype::RGB16},
    tags::Tag,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const WIDTH: u32 = 360;
const HEIGHT: u32 = 240;
const PIVOT: f64 = 0.18;
const MATCH: DVec3 = DVec3::new(0.22, 0.18, 0.14);

#[derive(Clone)]
struct Variant {
    name: String,
    label: String,
    contrast: f64,
    contrast_blend: f64,
    saturation: f64,
    density_amount: f64,
    saturation_blend: f64,
    density: Saturation,
}
impl Variant {
    fn identity() -> Self {
        Self {
            name: "identity".into(),
            label: "Identity".into(),
            contrast: 1.,
            contrast_blend: 0.,
            saturation: 1.,
            density_amount: 1.,
            saturation_blend: 0.,
            density: Saturation::Maximum,
        }
    }
    fn apply(&self, rgb: DVec3) -> operators::Result<DVec3> {
        let c = blend(
            contrast(rgb, self.contrast, PIVOT, Contrast::Rgb)?,
            contrast(rgb, self.contrast, PIVOT, Contrast::BlackOffsetRatios)?,
            self.contrast_blend,
        )?;
        blend(
            saturation(c, self.saturation, Saturation::ConstantY)?,
            saturation(c, self.density_amount, self.density)?,
            self.saturation_blend,
        )
    }
}

fn variants() -> Result<Vec<Variant>> {
    let mut out = vec![Variant::identity()];
    for (direction, amount) in [("down", 0.8), ("up", 1.2)] {
        for (i, t) in [0., 0.25, 0.5, 0.75, 1.].into_iter().enumerate() {
            out.push(Variant {
                name: format!("contrast-{direction}-{i}"),
                label: format!("Contrast {amount}; ratio proxy t={t}"),
                contrast: amount,
                contrast_blend: t,
                ..Variant::identity()
            });
        }
    }
    for (direction, amount) in [("down", 0.7), ("up", 1.3)] {
        for (name, model, matched) in [
            ("maximum", Saturation::Maximum, false),
            ("power", Saturation::DensityPower, false),
            ("power-matched", Saturation::DensityPower, true),
        ] {
            let density_amount = if matched {
                operators::matched_density_amount(MATCH, amount)?
            } else {
                amount
            };
            for (i, t) in [0., 0.25, 0.5, 0.75, 1.].into_iter().enumerate() {
                out.push(Variant {
                    name: format!("sat-{name}-{direction}-{i}"),
                    label: format!("Sat {amount}; {name} amount={density_amount:.6}; t={t}"),
                    saturation: amount,
                    density_amount,
                    saturation_blend: t,
                    density: model,
                    ..Variant::identity()
                });
            }
        }
    }
    for c in [0.8, 1.2] {
        for s in [0.7, 1.3] {
            out.push(Variant {
                name: format!("combined-{c}-{s}"),
                label: format!("Contrast {c}, Sat {s}; both blends 0.5 (power nominal)"),
                contrast: c,
                contrast_blend: 0.5,
                saturation: s,
                density_amount: s,
                saturation_blend: 0.5,
                density: Saturation::DensityPower,
            });
        }
    }
    Ok(out)
}

struct Fixture {
    name: &'static str,
    inputs: Vec<DVec3>,
    samples: usize,
}

fn synthetic() -> Fixture {
    let mut inputs = Vec::new();
    let weights = antitone::color::AP1_TO_XYZ.transpose().y_axis;
    for py in 0..HEIGHT {
        for px in 0..WIDTH {
            let u = px as f64 / (WIDTH - 1) as f64;
            let band = py / 40;
            let v = (py % 40) as f64 / 39.;
            let hue = |h: f64| {
                DVec3::new(
                    h.cos(),
                    (h - 2. * std::f64::consts::PI / 3.).cos(),
                    (h + 2. * std::f64::consts::PI / 3.).cos(),
                )
            };
            let rgb = match band {
                0 => DVec3::splat(-0.01 + u * 0.41),
                1 => DVec3::splat(2_f64.powf(-12. + u * 18.)),
                2 | 3 => {
                    let chromatic = hue(u * std::f64::consts::TAU);
                    let d = chromatic - DVec3::splat(y(chromatic));
                    (if band == 2 { 0.18 } else { 2. }) * (DVec3::ONE + (0.1 + v * 1.4) * d)
                }
                4 => DVec3::new(
                    0.2 + (u - 0.5) * 0.1,
                    0.2 - (u - 0.5) * 0.1,
                    0.02 + v * 0.17,
                ),
                _ => {
                    let d = 0.05 + v * 0.2;
                    DVec3::new(d, -d * weights.x / weights.y, 0.) + DVec3::splat((u - 0.5) * 0.0002)
                }
            };
            inputs.push(rgb);
        }
    }
    Fixture {
        name: "synthetic",
        inputs,
        samples: 1,
    }
}

fn rect(x: u32, y: u32, width: u32, height: u32) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn photos(root: &Path, directory: &Path, exposures: [f64; 3]) -> Result<Vec<Fixture>> {
    let mut white = Scan::open(&root.join("DSC04378.ARW"), None)?;
    let wb = white.fixed_wb;
    let dmax = white.reference(rect(4000, 2600, 128, 128))?;
    drop(white);
    let mut base = Scan::open(&root.join("DSC04393.ARW"), Some(wb))?;
    let calibration = Calibration {
        dmin: base.reference(rect(5000, 5660, 128, 32))?,
        dmax,
        input_floor: base.input_floor,
    }
    .validate()?;
    drop(base);
    let mut ring = Scan::open(&root.join("DSC04399.ARW"), Some(wb))?;
    let mut roll = RollSettings::new_grade();
    roll.balance = ring.roll_neutral(rect(3200, 3700, 64, 64), calibration, roll)?;
    drop(ring);
    let mut manifest = fs::File::create_new(directory.join("photographic-inputs.txt"))?;
    writeln!(
        manifest,
        "RAW root={}\nfixed WB={wb:?}\ncalibration={calibration:?}\nroll={roll:?}\nPhoto WB is default; per-photo exposure is recorded below, not fitted per model.\nDmin: DSC04393 (5000,5660,128,32), measured native camera clear film\nDmax: DSC04378 (4000,2600,128,128), diagnostic white-negative anchor from current RAW regression\nRoll neutral: DSC04399 (3200,3700,64,64), visually white ring\nNo photo WB fit, source correction, restored project grade or per-model exposure fitting.\nPicture rectangle=(192,96,8136,5424); 2 stratified original pixels/output cell; 360x240. Nonlinear operators and ACES precede target-linear averaging. Not settled/export reconstruction.",
        root.display()
    )?;
    let mut out = Vec::new();
    for ((name, source), exposure) in [
        ("dog", "DSC04393.ARW"),
        ("ring", "DSC04399.ARW"),
        ("canopy", "DSC04415.ARW"),
    ]
    .into_iter()
    .zip(exposures)
    {
        let photo = PhotoSettings {
            exposure,
            ..PhotoSettings::default()
        };
        let scan = Scan::open(&root.join(source), Some(wb))?;
        let pipeline = Pipeline::new(calibration, roll, photo, scan.camera_to_ap1)?;
        let mut inputs = Vec::new();
        for py in 0..HEIGHT {
            for px in 0..WIDTH {
                for fraction in [0.25, 0.75] {
                    let x = 192 + ((px as f64 + fraction) * 8136. / WIDTH as f64).floor() as u32;
                    let sy = 96 + ((py as f64 + fraction) * 5424. / HEIGHT as f64).floor() as u32;
                    inputs.push(pipeline.apply(scan.pixel(x, sy)?)?.rgb);
                }
            }
        }
        writeln!(
            manifest,
            "{name}: {source}; photo={photo:?}; camera={} camera_to_ap1={:?}",
            scan.camera_model, scan.camera_to_ap1
        )?;
        out.push(Fixture {
            name,
            inputs,
            samples: 2,
        });
    }
    Ok(out)
}

struct Icc(&'static [u8]);
impl tiff::encoder::TiffValue for Icc {
    const BYTE_LEN: u8 = 1;
    const FIELD_TYPE: tiff::tags::Type = tiff::tags::Type::UNDEFINED;
    fn count(&self) -> usize {
        self.0.len()
    }
    fn data(&self) -> std::borrow::Cow<'_, [u8]> {
        std::borrow::Cow::Borrowed(self.0)
    }
}

fn write_image(path: &Path, target: OutputTarget, pixels: &[Vec3]) -> Result<()> {
    let codes = target.encode_u16(pixels)?;
    let mut encoder = TiffEncoder::new(fs::File::create_new(path)?)?;
    let mut image = encoder.new_image::<RGB16>(WIDTH, HEIGHT)?;
    image
        .encoder()
        .write_tag(Tag::IccProfile, Icc(profiles::icc(target)))?;
    image.encoder().write_tag(
        Tag::ImageDescription,
        "Antitone research only; unchanged ACES 2; see adjacent manifest",
    )?;
    image.write_data(&codes.into_iter().flatten().collect::<Vec<_>>())?;
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let simple = args.first().is_some_and(|arg| arg == "--simple");
    let args = if simple { &args[1..] } else { &args[..] };
    if args.len() != 2 || simple && args[0] == "--synthetic" {
        return Err("Usage: antitone-contrast-saturation-probe [--simple] RAW_DIRECTORY|--synthetic NEW_OUTPUT_DIRECTORY (--simple requires RAWs)".into());
    }
    let directory = Path::new(&args[1]);
    fs::create_dir_all(
        directory
            .parent()
            .ok_or("output requires a parent directory")?,
    )?;
    fs::create_dir(directory)?;
    if simple {
        return simple::run(Path::new(&args[0]), directory);
    }
    let variants = variants()?;
    let mut fixtures = vec![synthetic()];
    if args[0] != "--synthetic" {
        fixtures.extend(photos(Path::new(&args[0]), directory, [2.5; 3])?);
    }
    let mut report = fs::File::create_new(directory.join("metrics.csv"))?;
    writeln!(
        report,
        "fixture,variant,target,samples,negative_ap1_pixels,mean_ap1_y,mean_ap1_chroma,mean_rendered_y"
    )?;
    let mut diagnostics = fs::File::create_new(directory.join("diagnostics.csv"))?;
    writeln!(diagnostics, "variant,r,g,b,out_r,out_g,out_b,Y,C")?;
    let weights = antitone::color::AP1_TO_XYZ.transpose().y_axis;
    for variant in &variants {
        for rgb in [
            DVec3::ZERO,
            DVec3::splat(PIVOT),
            MATCH,
            DVec3::new(0.2, -0.2 * weights.x / weights.y, 0.),
            DVec3::new(4., 0.3, 0.02),
        ] {
            let out = variant.apply(rgb)?;
            writeln!(
                diagnostics,
                "{},{},{},{},{},{},{},{},{}",
                variant.name,
                rgb.x,
                rgb.y,
                rgb.z,
                out.x,
                out.y,
                out.z,
                y(out),
                chroma(out)
            )?;
        }
    }
    let mut html = String::from(
        "<!doctype html><meta charset=utf-8><title>Contrast and saturation reference</title><style>body{background:#202020;color:#ddd;font:14px system-ui;margin:24px} .row{display:flex;gap:12px;overflow-x:auto}figure{margin:0;width:360px;flex-shrink:0}img{width:360px;height:240px}h2{margin-top:36px}figcaption{min-height:48px}a{color:#9cd}</style><h1>Contrast and saturation — reference candidates</h1><p>Identity is the fixed baseline. Contrast t=0 RGB ACEScct; t=1 black-offset-ratio proxy (not perceived saturation). Saturation t=0 constant AP1 Y; t=1 maximum-anchor or signed density power. Linear AP1 endpoint interpolation; unchanged ACES 2. No production defaults selected.</p><p>Images have embedded target profiles. Photographs: two stratified original pixels/output cell, not full-resolution reconstruction. Synthetic bands: signed neutral; HDR neutral; midtone hue/chroma; HDR hue/chroma; maximum crossover; near-zero signed Y. Scroll each row to compare blend progression. <a href='metrics.csv'>metrics</a> · <a href='diagnostics.csv'>diagnostics</a></p>",
    );
    for fixture in &fixtures {
        for (target_name, target) in [
            ("srgb", OutputTarget::Srgb),
            ("display-p3", OutputTarget::DisplayP3),
        ] {
            let renderer = CpuRenderer::new(target)?;
            let target_to_xyz = target.rgb_to_xyz();
            html.push_str(&format!("<h2>{} — {target_name}</h2>", fixture.name));
            for chunk in std::iter::once(&variants[..1]).chain(variants[1..].chunks(5)) {
                html.push_str("<div class=row>");
                for variant in chunk {
                    let working = fixture
                        .inputs
                        .iter()
                        .map(|rgb| variant.apply(*rgb))
                        .collect::<operators::Result<Vec<_>>>()?;
                    let rendered = renderer.render(&working)?;
                    let pixels: Vec<_> = rendered
                        .chunks_exact(fixture.samples)
                        .map(|p| {
                            (p.iter().map(|v| v.as_dvec3()).sum::<DVec3>() / fixture.samples as f64)
                                .as_vec3()
                        })
                        .collect();
                    let name = format!("{}-{target_name}-{}", fixture.name, variant.name);
                    write_image(&directory.join(format!("{name}.tif")), target, &pixels)?;
                    let count = working.len() as f64;
                    let rendered_y = rendered
                        .iter()
                        .map(|p| (target_to_xyz * p.as_dvec3()).y)
                        .sum::<f64>()
                        / count;
                    writeln!(
                        report,
                        "{},{},{target_name},{},{},{:.12e},{:.12e},{rendered_y:.12e}",
                        fixture.name,
                        variant.name,
                        working.len(),
                        working.iter().filter(|p| p.min_element() < 0.).count(),
                        working.iter().map(|p| y(*p)).sum::<f64>() / count,
                        working.iter().map(|p| chroma(*p)).sum::<f64>() / count
                    )?;
                    html.push_str(&format!("<figure><a href='{name}.png'><img loading=lazy src='{name}.png'></a><figcaption>{}</figcaption></figure>", variant.label));
                }
                html.push_str("</div><br>");
            }
            println!(
                "rendered {} {target_name}: {} variants",
                fixture.name,
                variants.len()
            );
        }
    }
    fs::write(directory.join("index.html"), html)?;
    fs::write(
        directory.join("run.txt"),
        format!(
            "OCIO={}\nfixtures={}\nvariants={}\nsize={WIDTH}x{HEIGHT}\npivot={PIVOT}\nstrength-match reference={MATCH:?}\nMatch is absolute AP1 opponent distance on one fixed patch, not perceived/display saturation or image matching.\nCandidate equations and ranges: docs/research/tools/contrast-saturation/README.md\n",
            ocio_version(),
            fixtures.len(),
            variants.len()
        ),
    )?;
    println!(
        "comparison gallery={}",
        directory.join("index.html").display()
    );
    Ok(())
}
