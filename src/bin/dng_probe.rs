//! Read-only direct-SDK diagnostic. No editor/source-store publication.

use std::path::PathBuf;
use std::time::Instant;

use antitone::dng::{Cancellation, INTERPRETATION, Options, Source};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1).peekable();
    let usage = "Usage: antitone-dng-probe [--threads 1..8] MAX_SENSOR_PIXELS FILE.DNG ...";
    let threads = if args.peek().is_some_and(|arg| arg == "--threads") {
        args.next();
        args.next()
            .ok_or(usage)?
            .to_str()
            .ok_or(usage)?
            .parse::<u32>()?
    } else {
        1
    };
    let max_pixels: u32 = args.next().ok_or(usage)?.to_str().ok_or(usage)?.parse()?;
    let paths: Vec<PathBuf> = args.map(PathBuf::from).collect();
    if max_pixels == 0 || paths.is_empty() {
        return Err(usage.into());
    }
    println!("{INTERPRETATION}");
    let cancel = Cancellation::default();
    let mut fixed_wb = None;
    for path in paths {
        let start = Instant::now();
        let source = Source::develop_with_threads(
            &path,
            Options {
                max_pixels,
                fixed_wb,
            },
            &cancel,
            threads,
        )?;
        fixed_wb = Some(source.fixed_wb());
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        let mut sum = [0_f64; 3];
        let mut camera_hash = blake3::Hasher::new();
        let mut original_hash = blake3::Hasher::new();
        for value in source.original_cfa() {
            original_hash.update(&value.to_le_bytes());
        }
        let [width, height] = source.dimensions();
        for rgb in source.camera_rgb() {
            for p in 0..3 {
                min[p] = min[p].min(rgb[p]);
                max[p] = max[p].max(rgb[p]);
                sum[p] += f64::from(rgb[p]);
                camera_hash.update(&rgb[p].to_bits().to_le_bytes());
            }
        }
        let mean = sum.map(|v| v / (f64::from(width) * f64::from(height)));
        println!("SOURCE {}", path.display());
        println!("native_timings={:?}", source.timings());
        println!(
            "camera_digest={} original_digest={}",
            camera_hash.finalize(),
            original_hash.finalize()
        );
        println!(
            "original={:?} active={:?} origin={:?} crop={:?} orientation={}",
            source.original_dimensions(),
            source.dimensions(),
            source.active_origin(),
            source.default_crop(),
            source.tiff_orientation()
        );
        println!(
            "capture_limits={:?} fixed_wb={:?} opcodes={:?}",
            source.capture_limits(),
            source.fixed_wb(),
            source.opcode_counts()
        );
        println!("camera_min={min:?} camera_max={max:?} camera_mean={mean:?}");
        println!(
            "camera_to_xyz_d50_columns={:?}",
            source.camera_to_xyz_d50().to_cols_array_2d()
        );
        println!(
            "interpretation_id={:02x?} retained_bytes={} elapsed_seconds={:.6}",
            source.interpretation_id(),
            source.retained_bytes(),
            start.elapsed().as_secs_f64()
        );
    }
    Ok(())
}
