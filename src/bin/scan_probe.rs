use antitone::{raw::Rect, scan::Scan};
use std::{error::Error, path::Path};

fn main() -> Result<(), Box<dyn Error>> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments.len() != 5 {
        return Err("Usage: antitone-scan-probe PHOTO.ARW x y width height".into());
    }
    let region = Rect {
        x: arguments[1].parse()?,
        y: arguments[2].parse()?,
        width: arguments[3].parse()?,
        height: arguments[4].parse()?,
    };
    let mut scan = Scan::open(Path::new(&arguments[0]), None)?;
    let first = scan.reference(region)?;
    let second = scan.reference(region)?;
    if first != second {
        return Err("cached measurements changed".into());
    }
    let (width, height, pixels) = scan.preview_source(32)?;
    for y in 0..height {
        for x in 0..width {
            let sx = scan.area.x
                + ((2 * u64::from(x) + 1) * u64::from(scan.area.width) / (2 * u64::from(width)))
                    as u32;
            let sy = scan.area.y
                + ((2 * u64::from(y) + 1) * u64::from(scan.area.height) / (2 * u64::from(height)))
                    as u32;
            let codes = pixels[(y * width + x) as usize];
            let camera =
                glam::DVec3::new(codes[0] as f64, codes[1] as f64, codes[2] as f64) / 65535.;
            if camera != scan.pixel(sx, sy)? {
                return Err("preview/full-resolution coordinate mismatch".into());
            }
        }
    }
    let (w, h) = scan.dimensions()?;
    if scan
        .reference(Rect {
            x: w,
            y: h,
            width: 1,
            height: 1,
        })
        .is_ok()
    {
        return Err("invalid rectangle accepted".into());
    }
    if scan.reference(region)? != first {
        return Err("failed pick changed source state".into());
    }
    println!(
        "cached_camera_image={w}x{h} reference={first:?} preview={width}x{height} exact_grid_centers=true failed_pick_no_mutation=true precision_floor={:.12e}",
        scan.input_floor
    );
    Ok(())
}
