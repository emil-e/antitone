use super::api::{processing::Calibration, raw::Rect, scan::Scan};
use std::path::PathBuf;

pub fn load() -> (Scan, Calibration) {
    let root = std::env::var_os("ANTITONE_RAW_SAMPLES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("samples"));
    let mut dmax = Scan::open(&root.join("DSC04378.ARW"), None).unwrap();
    let wb = dmax.fixed_wb;
    let matrix = dmax.camera_to_ap1;
    let model = dmax.camera_model.clone();
    let white = dmax
        .reference(Rect {
            x: 4000,
            y: 2600,
            width: 128,
            height: 128,
        })
        .unwrap();
    drop(dmax);
    let mut dmin = Scan::open(&root.join("DSC04397.ARW"), Some(wb)).unwrap();
    assert_eq!(dmin.camera_to_ap1, matrix);
    assert_eq!(dmin.camera_model, model);
    let base = dmin
        .reference(Rect {
            x: 4000,
            y: 5644,
            width: 128,
            height: 32,
        })
        .unwrap();
    drop(dmin);
    let train = Scan::open(&root.join("DSC04524.ARW"), Some(wb)).unwrap();
    assert_eq!(train.camera_to_ap1, matrix);
    assert_eq!(train.camera_model, model);
    assert_eq!(train.fixed_wb, wb);
    assert_eq!(train.input_floor, 1. / 65535.);
    let calibration = Calibration {
        dmin: base,
        dmax: white,
        input_floor: train.input_floor,
    }
    .validate()
    .unwrap();
    (train, calibration)
}
