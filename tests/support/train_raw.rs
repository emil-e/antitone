use super::api::{processing::Calibration, raw::Rect, scan::DevelopedSource};
use std::{path::PathBuf, sync::Arc};

pub fn load() -> (Arc<DevelopedSource>, Calibration) {
    let root = std::env::var_os("ANTITONE_RAW_SAMPLES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("samples"));
    // Calibration and scan samples use the Negative stage's unity-WB development basis.
    let dmax = DevelopedSource::open(&root.join("DSC04378.ARW"), Some([1.; 4])).unwrap();
    let wb = dmax.white_balance();
    let matrix = dmax.camera_to_ap1();
    let model = dmax.camera_model().to_owned();
    let white = super::api::sampling::camera::reference(
        &dmax,
        None,
        Rect {
            x: 4000,
            y: 2600,
            width: 128,
            height: 128,
        },
    )
    .unwrap();
    drop(dmax);
    let dmin = DevelopedSource::open(&root.join("DSC04397.ARW"), Some(wb)).unwrap();
    assert_eq!(dmin.camera_to_ap1(), matrix);
    assert_eq!(dmin.camera_model(), model);
    let base = super::api::sampling::camera::reference(
        &dmin,
        None,
        Rect {
            x: 4000,
            y: 5644,
            width: 128,
            height: 32,
        },
    )
    .unwrap();
    drop(dmin);
    let train = DevelopedSource::open(&root.join("DSC04524.ARW"), Some(wb)).unwrap();
    assert_eq!(train.camera_to_ap1(), matrix);
    assert_eq!(train.camera_model(), model);
    assert_eq!(train.white_balance(), wb);
    let calibration = Calibration {
        dmin: base,
        dmax: white,
    }
    .validate()
    .unwrap();
    (train, calibration)
}
