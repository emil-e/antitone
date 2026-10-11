use antitone::{
    export::{self, ExrPrecision, ImageSize, Overwrite},
    processing::{Calibration, PhotoSettings, Pipeline, RollSettings},
    raw::{RawFile, Rect},
    rendering::OutputTarget,
};
use glam::DVec3;
use std::{
    error::Error,
    path::{Path, PathBuf},
    time::Instant,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn rect(arguments: &[String], offset: usize) -> Result<Rect> {
    Ok(Rect {
        x: arguments[offset].parse()?,
        y: arguments[offset + 1].parse()?,
        width: arguments[offset + 2].parse()?,
        height: arguments[offset + 3].parse()?,
    })
}
fn reference(path: &Path, area: Rect, wb: [f64; 4]) -> Result<DVec3> {
    let mut raw = RawFile::open(path)?;
    let diagnostics = raw.region_sensor_diagnostics(area)?;
    if diagnostics.at_white.iter().any(|v| *v > 0) || diagnostics.below_black.iter().any(|v| *v > 0)
    {
        return Err("reference contains sensor clipping; choose another rectangle".into());
    }
    let developed = raw.develop_camera(wb)?;
    let mean = developed.sample_mean(area, glam::DMat3::IDENTITY)?;
    println!("reference={path:?} rectangle={area:?} camera_mean={mean:?}");
    Ok(mean)
}

fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    if a.len() != 12 {
        return Err("Usage: antitone-export-probe DMIN.ARW x y width height DMAX.ARW x y width height PHOTO.ARW OUTPUT_DIRECTORY (explicit diagnostic calibration; exports full developed image, default settings)".into());
    }
    let start = Instant::now();
    let dmax_source = RawFile::open(Path::new(&a[5]))?;
    let wb = dmax_source.metadata.cam_mul;
    let floor = antitone::processing::CAMERA_INPUT_FLOOR;
    drop(dmax_source);
    let calibration = Calibration {
        dmin: reference(Path::new(&a[0]), rect(&a, 1)?, wb)?,
        dmax: reference(Path::new(&a[5]), rect(&a, 6)?, wb)?,
        input_floor: floor,
    }
    .validate()?;
    println!(
        "fixed_development_wb={wb:?} precision_floor={floor:.12e} (not a noise estimate) calibration_seconds={:.2}",
        start.elapsed().as_secs_f64()
    );
    let source = Path::new(&a[10]);
    let directory = PathBuf::from(&a[11]);
    std::fs::create_dir_all(&directory)?;
    let stem = source
        .file_stem()
        .ok_or("source has no filename")?
        .to_string_lossy();
    let mut raw = RawFile::open(source)?;
    let matrix = raw.metadata.camera_to_ap1()?;
    let pipeline = Pipeline::new(
        calibration,
        RollSettings::default(),
        PhotoSettings::default(),
        matrix,
    )?;
    let developed = raw.develop_camera(wb)?;
    let size = ImageSize {
        width: developed.width,
        height: developed.height,
    };
    println!(
        "developed={}x{} source={source:?} elapsed_seconds={:.2}",
        size.width,
        size.height,
        start.elapsed().as_secs_f64()
    );
    let pixel = |x, y| pipeline.apply(developed.pixel(x, y)?).map(|p| p.rgb);
    for (suffix, target) in [
        ("display-p3", OutputTarget::DisplayP3),
        ("srgb", OutputTarget::Srgb),
    ] {
        let path = directory.join(format!("{stem}-{suffix}.tif"));
        let export_start = Instant::now();
        export::tiff(
            &path,
            size,
            target,
            Overwrite::Refuse,
            RollSettings::default().response,
            pixel,
        )?;
        println!(
            "written={path:?} bytes={} seconds={:.2}",
            std::fs::metadata(&path)?.len(),
            export_start.elapsed().as_secs_f64()
        );
    }
    let path = directory.join(format!("{stem}.exr"));
    let export_start = Instant::now();
    export::exr(
        &path,
        size,
        ExrPrecision::Half,
        Overwrite::Refuse,
        RollSettings::default().response,
        pixel,
    )?;
    println!(
        "written={path:?} bytes={} seconds={:.2} total_seconds={:.2}",
        std::fs::metadata(&path)?.len(),
        export_start.elapsed().as_secs_f64(),
        start.elapsed().as_secs_f64()
    );
    Ok(())
}
