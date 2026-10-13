#![cfg(all(feature = "raw", target_os = "macos"))]
use antitone::{
    processing::RollSettings,
    raw::Rect,
    roll_analysis::{self, FrameSamples, MAX_SAMPLES_PER_FRAME},
    scan::Scan,
};
use glam::DVec3;
use std::{fs, path::PathBuf};

#[test]
fn authored_roll_samples_real_capture_footprints_and_estimates_a_shared_baseline() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw");
    let paths = [root.join("roll-0.dng"), root.join("roll-1.dng")];
    let originals = paths.each_ref().map(|path| fs::read(path).unwrap());
    let film = Rect {
        x: 8,
        y: 8,
        width: 496,
        height: 496,
    };
    let picture = Rect {
        x: 96,
        y: 96,
        width: 320,
        height: 320,
    };
    let mut owned = Vec::new();
    let mut matrix = None;
    for path in &paths {
        let mut scan = Scan::open(path, None).unwrap();
        assert_eq!(scan.fixed_wb, [1.; 4]);
        assert_eq!(scan.area, film);
        if let Some(matrix) = matrix {
            assert_eq!(scan.camera_to_ap1, matrix);
        }
        matrix = Some(scan.camera_to_ap1);
        let film_samples = scan
            .analysis_samples(film, MAX_SAMPLES_PER_FRAME, || false)
            .unwrap()
            .unwrap();
        let picture_samples = scan
            .analysis_samples(picture, MAX_SAMPLES_PER_FRAME, || false)
            .unwrap()
            .unwrap();
        for (samples, count) in [(&film_samples, 3844), (&picture_samples, 1600)] {
            assert_eq!(samples.attempted, count);
            assert_eq!(samples.pixels.len(), count);
            assert_eq!(samples.rejected_capture, 0);
            assert_eq!(samples.rejected_development, 0);
            assert_eq!(samples.rejected_floor, 0);
        }
        assert!(
            scan.analysis_samples(picture, MAX_SAMPLES_PER_FRAME, || true)
                .unwrap()
                .is_none()
        );
        owned.push((film_samples.pixels, picture_samples.pixels));
    }
    let frames: Vec<_> = owned
        .iter()
        .map(|(film, picture)| FrameSamples { film, picture })
        .collect();
    let roll = RollSettings::new_grade();
    let proposal = roll_analysis::estimate(
        &frames,
        antitone::processing::CAMERA_INPUT_FLOOR,
        matrix.unwrap(),
        roll,
        || false,
    )
    .unwrap()
    .unwrap();
    assert!(
        (proposal.dmin - DVec3::splat(3000. / 4031.))
            .abs()
            .max_element()
            <= 2. / 65535.
    );
    assert!(proposal.dmax.min_element() > 192. / 4031.);
    assert!(proposal.dmax.max_element() < 300. / 4031.);
    assert_eq!(proposal.roll.gamma, roll.gamma);
    assert_eq!(proposal.roll.response, roll.response);
    assert_eq!(proposal.photos.len(), 2);
    for statistics in &proposal.frames {
        assert!(statistics.balance_samples >= 256);
        assert!(statistics.exposure_samples >= 512);
    }
    // The denser second negative is brighter after inversion and needs less exposure.
    assert!(proposal.photos[0].exposure > proposal.photos[1].exposure);
    assert!((proposal.photos[0].exposure + proposal.photos[1].exposure).abs() < 1e-12);
    assert!(
        roll_analysis::estimate(
            &frames,
            antitone::processing::CAMERA_INPUT_FLOOR,
            matrix.unwrap(),
            roll,
            || true
        )
        .unwrap()
        .is_none()
    );
    for (path, original) in paths.iter().zip(originals) {
        assert_eq!(fs::read(path).unwrap(), original);
    }
}
