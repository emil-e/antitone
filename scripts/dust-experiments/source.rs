//! Private research extraction/rendering through the production RAW and colour pipeline.
use antitone::{
    dust_repair::{
        DonorChoice, Repair, RepairId, RepairOrigin, SupportMask, evaluate, heal_ranked,
    },
    export::{self, ImageSize, Overwrite},
    geometry::SourceRegion,
    processing::{CAMERA_INPUT_FLOOR, Calibration, PhotoSettings, Pipeline, RollSettings},
    raw::{RawFile, Rect},
    rendering::{CpuRenderer, OutputTarget},
};
use glam::DVec3;
use serde_json::json;
use std::{error::Error, fs, io::Write, path::Path, time::Instant};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn rect(a: &[String], i: usize) -> Result<Rect> {
    Ok(Rect {
        x: a[i].parse()?,
        y: a[i + 1].parse()?,
        width: a[i + 2].parse()?,
        height: a[i + 3].parse()?,
    })
}

fn reference(
    path: &str,
    area: Rect,
    wb: [f64; 4],
    basis: &str,
    matrix: glam::DMat3,
) -> Result<DVec3> {
    let mut raw = RawFile::open(Path::new(path))?;
    if raw.metadata.camera_model() != basis || raw.metadata.camera_to_ap1()? != matrix {
        return Err("reference camera basis differs".into());
    }
    raw.develop_camera(wb)?;
    let d = raw.capture_eligibility(area)?;
    if d.at_white || d.below_black {
        return Err("sensor-clipped calibration reference".into());
    }
    Ok(raw
        .camera_image()?
        .sample_mean(area, glam::DMat3::IDENTITY)?)
}

fn render(pixels: &[DVec3], width: u32, height: u32, output: &Path) -> Result<()> {
    let target = OutputTarget::Srgb;
    let rendered = CpuRenderer::new(target)?.render(pixels)?;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    write!(
        f,
        "P6\n# ACES 2 sRGB diagnostic, encoded, no embedded ICC\n{width} {height}\n255\n"
    )?;
    let mut codes = Vec::with_capacity(pixels.len() * 3);
    for p in rendered {
        for c in p.to_array() {
            codes.push((target.encode(f64::from(c))? * 255.0).round() as u8);
        }
    }
    f.write_all(&codes)?;
    Ok(())
}

fn camera_frame(photo: &Path, wb_reference: &Path, directory: &Path) -> Result<()> {
    let start = Instant::now();
    let reference = RawFile::open(wb_reference)?;
    let wb = reference.metadata.cam_mul;
    let basis = reference.metadata.camera_model();
    let matrix = reference.metadata.camera_to_ap1()?;
    drop(reference);
    let mut raw = RawFile::open(photo)?;
    let metadata = raw.metadata;
    if metadata.camera_model() != basis || metadata.camera_to_ap1()? != matrix {
        return Err("source camera basis differs".into());
    }
    let area = metadata.image_area()?;
    let developed = raw.develop_camera(wb)?;
    fs::create_dir(directory)?;
    let mut output = std::io::BufWriter::new(
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("camera.rgb16"))?,
    );
    let mut row = Vec::with_capacity(area.width as usize * 6);
    for y in area.y..area.y + area.height {
        row.clear();
        for x in area.x..area.x + area.width {
            for channel in &developed.pixel_codes(x, y)?[..3] {
                row.extend(channel.to_le_bytes());
            }
        }
        output.write_all(&row)?;
    }
    output.flush()?;
    let info = json!({
        "source": fs::canonicalize(photo)?, "camera": basis, "libraw": antitone::raw::version(),
        "region": [area.x, area.y, area.width, area.height], "width": area.width, "height": area.height,
        "fixed_wb": wb, "wb_reference": fs::canonicalize(wb_reference)?, "flip": metadata.flip,
        "camera_to_ap1_columns": matrix.to_cols_array(),
        "representation": "whole metadata area, native-camera RGB16 triples, no inversion or grade",
        "seconds": start.elapsed().as_secs_f64()
    });
    fs::write(
        directory.join("frame.json"),
        serde_json::to_vec_pretty(&info)?,
    )?;
    println!(
        "camera_frame={} {}x{} seconds={:.2}",
        directory.display(),
        area.width,
        area.height,
        start.elapsed().as_secs_f64()
    );
    Ok(())
}

#[derive(serde::Deserialize)]
struct CropProcessing {
    width: u32,
    height: u32,
    processing_model: u32,
    stride: u32,
    dmin: [f64; 3],
    dmax: [f64; 3],
    camera_to_ap1_columns: [f64; 9],
    roll: RollSettings,
    photo: PhotoSettings,
}

fn export_tiff(source: &Path, working_path: &Path, output: &Path, target: &str) -> Result<()> {
    let info: CropProcessing = serde_json::from_slice(&fs::read(source.join("source.json"))?)?;
    let pixels = u64::from(info.width) * u64::from(info.height);
    if info.processing_model != antitone::PROCESSING_MODEL_VERSION
        || info.stride != 1
        || !(1..=4_000_000).contains(&pixels)
    {
        return Err("unsupported model, survey proxy or unbounded TIFF crop".into());
    }
    let target = match target {
        "srgb" => OutputTarget::Srgb,
        "display-p3" => OutputTarget::DisplayP3,
        _ => return Err("TIFF target must be srgb or display-p3".into()),
    };
    let bytes = fs::read(working_path)?;
    if bytes.len() != pixels as usize * 12 {
        return Err("float RGB dimensions disagree".into());
    }
    let values = bytes.as_chunks::<12>().0;
    export::tiff(
        output,
        ImageSize {
            width: info.width,
            height: info.height,
        },
        target,
        Overwrite::Refuse,
        info.roll.response,
        |x, y| {
            let p = &values[(y * info.width + x) as usize];
            Ok(DVec3::from_array(std::array::from_fn(|c| {
                f32::from_le_bytes(p[c * 4..c * 4 + 4].try_into().unwrap()) as f64
            })))
        },
    )?;
    println!(
        "tiff={} {}x{} target={target:?} inspection_ev=0",
        output.display(),
        info.width,
        info.height
    );
    Ok(())
}

fn rust_repair(
    input: &Path,
    width: u32,
    height: u32,
    mask_path: &Path,
    output: &Path,
    donor: DonorChoice,
    bounded: bool,
) -> Result<()> {
    if output.exists() {
        return Err("repair output already exists".into());
    }
    let count = width
        .checked_mul(height)
        .filter(|count| (1..=4_000_000).contains(count))
        .ok_or("invalid repair dimensions")? as usize;
    let bytes = fs::read(input)?;
    let mask = fs::read(mask_path)?;
    if bytes.len() != count * 12 || mask.len() != count {
        return Err("float RGB or mask dimensions disagree".into());
    }
    let pixels = bytes
        .as_chunks::<12>()
        .0
        .iter()
        .map(|pixel| {
            std::array::from_fn(|channel| {
                f32::from_le_bytes(pixel[channel * 4..channel * 4 + 4].try_into().unwrap())
            })
        })
        .collect::<Vec<_>>();
    let mask = mask.into_iter().map(|value| value != 0).collect::<Vec<_>>();
    let (repaired, donors) = if bounded {
        let selected = mask
            .iter()
            .enumerate()
            .filter_map(|(index, selected)| selected.then_some(index))
            .collect::<Vec<_>>();
        let min_x = selected
            .iter()
            .map(|index| index % width as usize)
            .min()
            .ok_or("empty repair mask")? as u32;
        let max_x = selected
            .iter()
            .map(|index| index % width as usize)
            .max()
            .unwrap() as u32;
        let min_y = selected
            .iter()
            .map(|index| index / width as usize)
            .min()
            .unwrap() as u32;
        let max_y = selected
            .iter()
            .map(|index| index / width as usize)
            .max()
            .unwrap() as u32;
        let support_width = max_x - min_x + 1;
        let support_height = max_y - min_y + 1;
        let mut bits = vec![false; (support_width * support_height) as usize];
        for index in selected {
            let x = index as u32 % width;
            let y = index as u32 / width;
            bits[((y - min_y) * support_width + x - min_x) as usize] = true;
        }
        let repair = Repair {
            id: RepairId(1),
            enabled: true,
            origin: RepairOrigin::Manual,
            support: SupportMask::new([min_x, min_y], [support_width, support_height], bits)?,
            donor,
        };
        let evaluated = evaluate(
            &repair,
            SourceRegion {
                x: 0,
                y: 0,
                width,
                height,
            },
            |[x, y]| Ok(pixels[(y * width + x) as usize]),
        )?;
        let repaired = pixels
            .iter()
            .enumerate()
            .map(|(index, pixel)| {
                evaluated.pixel([index as u32 % width, index as u32 / width], *pixel)
            })
            .collect();
        (repaired, evaluated.donors)
    } else {
        heal_ranked(&pixels, [width, height], &mask, donor)?
    };
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    for pixel in repaired {
        for channel in pixel {
            temporary.write_all(&channel.to_le_bytes())?;
        }
    }
    temporary.flush()?;
    temporary.persist_noclobber(output)?;
    println!(
        "{}",
        json!({"donors": donors.iter().map(|donor| json!({
        "target": donor.target, "source": donor.source, "context_mse": donor.context_mse,
        "paired_context_pixels": donor.paired_context_pixels,
        "matching_score": donor.matching_score,
    })).collect::<Vec<_>>() })
    );
    Ok(())
}

fn develop_repair(
    source: &Path,
    camera_path: &Path,
    mask_path: &Path,
    output: &Path,
) -> Result<()> {
    let info: CropProcessing = serde_json::from_slice(&fs::read(source.join("source.json"))?)?;
    let pixels = u64::from(info.width) * u64::from(info.height);
    if info.processing_model != antitone::PROCESSING_MODEL_VERSION
        || info.stride != 1
        || !(1..=4_000_000).contains(&pixels)
    {
        return Err("unsupported model, survey proxy or unbounded development crop".into());
    }
    let camera = fs::read(camera_path)?;
    let mask = fs::read(mask_path)?;
    let mut working = fs::read(source.join("working.ap1-f32"))?;
    let n = pixels as usize;
    if camera.len() != n * 12 || working.len() != n * 12 || mask.len() != n {
        return Err("camera, mask and working dimensions disagree".into());
    }
    let pipeline = Pipeline::new(
        Calibration {
            dmin: DVec3::from_array(info.dmin),
            dmax: DVec3::from_array(info.dmax),
            input_floor: CAMERA_INPUT_FLOOR,
        },
        info.roll,
        info.photo,
        glam::DMat3::from_cols_array(&info.camera_to_ap1_columns),
    )?;
    let mut floor_pixels = 0;
    let mut floor_channels = [0_usize; 3];
    for (i, (codes, &selected)) in camera.as_chunks::<12>().0.iter().zip(&mask).enumerate() {
        if selected == 0 {
            continue;
        }
        let value = DVec3::from_array(std::array::from_fn(|c| {
            f64::from(f32::from_le_bytes(
                codes[c * 4..c * 4 + 4].try_into().unwrap(),
            ))
        }));
        let positive = pipeline.apply(value)?;
        floor_pixels += usize::from(positive.floored.iter().any(|v| *v));
        for (c, count) in floor_channels.iter_mut().enumerate() {
            *count += usize::from(positive.floored[c]);
            let narrowed = positive.rgb[c] as f32;
            if !narrowed.is_finite() {
                return Err("repaired AP1 exceeds finite f32 storage".into());
            }
            let offset = i * 12 + c * 4;
            working[offset..offset + 4].copy_from_slice(&narrowed.to_le_bytes());
        }
    }
    // Develop only selected pixels: normalizing RGB16 to f32 must not perturb known AP1.
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?
        .write_all(&working)?;
    println!(
        "{}",
        json!({"floor_pixels": floor_pixels, "floor_channels": floor_channels,
        "masked_pixels": mask.iter().filter(|&&v| v != 0).count()})
    );
    Ok(())
}

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.first().map(String::as_str) == Some("tiff") && a.len() == 5 {
        return export_tiff(Path::new(&a[1]), Path::new(&a[2]), Path::new(&a[3]), &a[4]);
    }
    if matches!(
        a.first().map(String::as_str),
        Some("rust-repair" | "rust-evaluate")
    ) && matches!(a.len(), 6 | 8)
    {
        let bounded = a[0] == "rust-evaluate";
        let donor = if a.len() == 8 {
            DonorChoice::Offset([a[6].parse()?, a[7].parse()?])
        } else {
            DonorChoice::Automatic
        };
        return rust_repair(
            Path::new(&a[1]),
            a[2].parse()?,
            a[3].parse()?,
            Path::new(&a[4]),
            Path::new(&a[5]),
            donor,
            bounded,
        );
    }
    if a.first().map(String::as_str) == Some("develop") && a.len() == 5 {
        return develop_repair(
            Path::new(&a[1]),
            Path::new(&a[2]),
            Path::new(&a[3]),
            Path::new(&a[4]),
        );
    }
    if a.first().map(String::as_str) == Some("camera") && a.len() == 4 {
        return camera_frame(Path::new(&a[1]), Path::new(&a[2]), Path::new(&a[3]));
    }
    if a.first().map(String::as_str) == Some("render") && matches!(a.len(), 5 | 6) {
        let width: u32 = a[2].parse()?;
        let height: u32 = a[3].parse()?;
        let bytes = fs::read(&a[1])?;
        if bytes.len() != width as usize * height as usize * 12 {
            return Err("float RGB dimensions disagree".into());
        }
        let inspection_ev: f64 = if a.len() == 6 { a[5].parse()? } else { 0.0 };
        let gain = antitone::processing::exposure_multiplier(inspection_ev)?;
        println!("render inspection_ev={inspection_ev} (input working data unchanged)");
        let pixels = bytes
            .as_chunks::<12>()
            .0
            .iter()
            .map(|p| {
                DVec3::from_array(std::array::from_fn(|c| {
                    f32::from_le_bytes(p[c * 4..c * 4 + 4].try_into().unwrap()) as f64
                }))
            })
            .map(|p| p * gain)
            .collect::<Vec<_>>();
        return render(&pixels, width, height, Path::new(&a[4]));
    }
    if a.first().map(String::as_str) != Some("extract") || !matches!(a.len(), 14 | 18) {
        return Err(
            "Usage: antitone-dust-source-probe tiff SOURCE_DIRECTORY INPUT.ap1-f32 NEW_OUTPUT.tif srgb|display-p3\n       antitone-dust-source-probe develop SOURCE_DIRECTORY CAMERA_F32 MASK_U8 NEW_AP1\n       antitone-dust-source-probe camera PHOTO.ARW WB_REFERENCE.ARW NEW_DIRECTORY\n       antitone-dust-source-probe extract DMIN.ARW x y w h DMAX.ARW x y w h PHOTO.ARW NEW_DIRECTORY MAX_SIDE [x y w h]\n       antitone-dust-source-probe render INPUT.ap1-f32 WIDTH HEIGHT NEW_OUTPUT.ppm [INSPECTION_EV]"
                .into(),
        );
    }
    let a = &a[1..];
    let start = Instant::now();
    let reference_raw = RawFile::open(Path::new(&a[5]))?;
    let wb = reference_raw.metadata.cam_mul;
    let basis = reference_raw.metadata.camera_model();
    let basis_matrix = reference_raw.metadata.camera_to_ap1()?;
    drop(reference_raw);
    let dmin = reference(&a[0], rect(a, 1)?, wb, &basis, basis_matrix)?;
    let dmax = reference(&a[5], rect(a, 6)?, wb, &basis, basis_matrix)?;
    let calibration = Calibration {
        dmin,
        dmax,
        input_floor: CAMERA_INPUT_FLOOR,
    }
    .validate()?;
    let roll = RollSettings::new_grade();
    let photo = PhotoSettings::default();
    let mut raw = RawFile::open(Path::new(&a[10]))?;
    let metadata = raw.metadata;
    if metadata.camera_model() != basis || metadata.camera_to_ap1()? != basis_matrix {
        return Err("source camera basis differs".into());
    }
    let region = if a.len() == 17 {
        rect(a, 13)?
    } else {
        metadata.image_area()?
    };
    region.validate()?;
    if region
        .x
        .checked_add(region.width)
        .is_none_or(|x| x > metadata.width)
        || region
            .y
            .checked_add(region.height)
            .is_none_or(|y| y > metadata.height)
    {
        return Err("region outside developed source".into());
    }
    let max_side: u32 = a[12].parse()?;
    let stride = if max_side == 0 {
        1
    } else {
        region.width.max(region.height).div_ceil(max_side).max(1)
    };
    let width = region.width.div_ceil(stride);
    let height = region.height.div_ceil(stride);
    if u64::from(width) * u64::from(height) > 4_000_000 {
        return Err("extract bounded crops/previews, not full-frame buffers".into());
    }
    let pipeline = Pipeline::new(calibration, roll, photo, basis_matrix)?;
    let developed = raw.develop_camera(wb)?;
    let mut camera = Vec::with_capacity(width as usize * height as usize * 6);
    let mut working = Vec::with_capacity(width as usize * height as usize);
    let mut ap1 = Vec::with_capacity(width as usize * height as usize * 12);
    let mut floored = 0;
    for y in 0..height {
        for x in 0..width {
            let sx = region.x + (x * stride + stride / 2).min(region.width - 1);
            let sy = region.y + (y * stride + stride / 2).min(region.height - 1);
            for c in &developed.pixel_codes(sx, sy)?[..3] {
                camera.extend(c.to_le_bytes());
            }
            let positive = pipeline.apply(developed.pixel(sx, sy)?)?;
            floored += usize::from(positive.floored.iter().any(|v| *v));
            working.push(positive.rgb);
            for c in positive.rgb.to_array() {
                ap1.extend((c as f32).to_le_bytes());
            }
        }
    }
    let directory = Path::new(&a[11]);
    fs::create_dir(directory)?;
    fs::write(directory.join("camera.rgb16"), camera)?;
    fs::write(directory.join("working.ap1-f32"), ap1)?;
    render(&working, width, height, &directory.join("original.ppm"))?;
    let info = json!({
        "source": fs::canonicalize(&a[10])?, "camera": basis, "libraw": antitone::raw::version(),
        "processing_model": antitone::PROCESSING_MODEL_VERSION, "source_width": metadata.width,
        "source_height": metadata.height, "flip": metadata.flip, "region": [region.x, region.y, region.width, region.height],
        "stride": stride, "width": width, "height": height, "fixed_wb": wb,
        "camera_to_ap1_columns": basis_matrix.to_cols_array(), "dmin": dmin.to_array(), "dmax": dmax.to_array(),
        "references": { "dmin": [a[0].clone(), a[1].clone(), a[2].clone(), a[3].clone(), a[4].clone()],
                        "dmax": [a[5].clone(), a[6].clone(), a[7].clone(), a[8].clone(), a[9].clone()] },
        "roll": roll, "photo": photo, "floor_pixels": floored,
        "calibration_status": "explicit diagnostic references, not accepted photographic roll calibration",
        "float_storage": "little-endian RGB f32 AP1, signed/HDR, no clamp",
        "preview": "nearest-source survey only; stride=1 crops are native-resolution",
        "seconds": start.elapsed().as_secs_f64()
    });
    fs::write(
        directory.join("source.json"),
        serde_json::to_vec_pretty(&info)?,
    )?;
    println!(
        "{} {}x{} stride={} floor_pixels={} seconds={:.2}",
        directory.display(),
        width,
        height,
        stride,
        floored,
        start.elapsed().as_secs_f64()
    );
    Ok(())
}
