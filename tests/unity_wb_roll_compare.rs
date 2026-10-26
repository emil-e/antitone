#![cfg(feature = "editor")]
//! Explicit private-roll check that unity WB and the former shared as-shot basis produce the
//! same analyzed positive look. The source project is read-only and supplied through the
//! environment; it is not a repository fixture.

use antitone::{
    color::AP1_TO_XYZ,
    project::{Project, UNITY_WB},
    raw::Rect,
    roll_analysis::{FrameSamples, MAX_SAMPLES_PER_FRAME, estimate},
    scan::Scan,
};
use glam::DVec3;
use std::path::PathBuf;

fn samples(
    scan: &mut Scan,
    area: Rect,
    areas: &antitone::scan_regions::Areas,
) -> (Vec<DVec3>, Vec<DVec3>) {
    let active = scan.area;
    let film = scan
        .analysis_samples_in(
            area,
            MAX_SAMPLES_PER_FRAME,
            |f| areas.contains_footprint(f, active, false),
            || false,
        )
        .unwrap()
        .unwrap()
        .pixels;
    let picture = scan
        .analysis_samples_in(
            area,
            MAX_SAMPLES_PER_FRAME,
            |f| areas.contains_footprint(f, active, true),
            || false,
        )
        .unwrap()
        .unwrap()
        .pixels;
    (film, picture)
}

#[test]
#[ignore = "explicit private unity-WB roll; set ANTITONE_UNITY_WB_PROJECT"]
fn unity_wb_roll_analysis_matches_as_shot_look() {
    let project_path = PathBuf::from(
        std::env::var_os("ANTITONE_UNITY_WB_PROJECT").expect("set ANTITONE_UNITY_WB_PROJECT"),
    );
    let project = Project::load(&project_path).unwrap();
    assert!(project.input.is_negative());
    let first = project.order.first().copied().unwrap();
    let first_path = project.source(first).unwrap().path.clone();
    let first_shot = Scan::open(&first_path, None).unwrap();
    let as_shot_wb = first_shot.fixed_wb;
    assert_ne!(as_shot_wb, UNITY_WB);
    let matrix = first_shot.camera_to_ap1;
    drop(first_shot);

    let mut unity_samples = (Vec::new(), Vec::new(), 0f64);
    let mut as_shot_samples = (Vec::new(), Vec::new(), 0f64);
    for id in &project.order {
        let source = project.source(*id).unwrap();
        let entry = project
            .analysis_regions
            .iter()
            .find(|entry| entry.source == *id)
            .unwrap();
        let area = Rect::from(source.development.active_area);
        for (target, wb) in [
            (&mut unity_samples, Some(UNITY_WB)),
            (&mut as_shot_samples, Some(as_shot_wb)),
        ] {
            let mut scan = Scan::open(&source.path, wb).unwrap();
            assert_eq!(scan.camera_to_ap1, matrix);
            let floor = scan.effective_floor();
            let (film, picture) = samples(&mut scan, area, &entry.areas);
            target.0.push(film);
            target.1.push(picture);
            target.2 = target.2.max(floor);
        }
    }

    let unity_frames: Vec<_> = (0..project.order.len())
        .map(|i| FrameSamples {
            film: &unity_samples.0[i],
            picture: &unity_samples.1[i],
        })
        .collect();
    let as_shot_frames: Vec<_> = (0..project.order.len())
        .map(|i| FrameSamples {
            film: &as_shot_samples.0[i],
            picture: &as_shot_samples.1[i],
        })
        .collect();
    let current = Project::default().input;
    let unity = estimate(&unity_frames, unity_samples.2, matrix, current, || false)
        .unwrap()
        .unwrap();
    let as_shot = estimate(&as_shot_frames, as_shot_samples.2, matrix, current, || {
        false
    })
    .unwrap()
    .unwrap();
    let un = unity.input.negative_input().unwrap();
    let an = as_shot.input.negative_input().unwrap();
    println!(
        "unity: dmin={:?} dmax={:?} gamma_balance={:?} roll_wb={:?} roll_ev={} max_photo_ev={}",
        un.calibration.dmin,
        un.calibration.dmax,
        un.balance,
        unity.roll.wb,
        unity.roll.exposure,
        unity.photos.iter().map(|p| p.exposure).fold(0f64, f64::max)
    );
    println!(
        "as-shot: dmin={:?} dmax={:?} gamma_balance={:?} roll_wb={:?} roll_ev={} max_photo_ev={}",
        an.calibration.dmin,
        an.calibration.dmax,
        an.balance,
        as_shot.roll.wb,
        as_shot.roll.exposure,
        as_shot
            .photos
            .iter()
            .map(|p| p.exposure)
            .fold(0f64, f64::max)
    );
    println!(
        "dmin ratio={:?} dmax ratio={:?}",
        an.calibration.dmin / un.calibration.dmin,
        an.calibration.dmax / un.calibration.dmax
    );

    let mut luminance_ratio = Vec::new();
    let mut chroma_difference = Vec::new();
    for i in 0..project.order.len() {
        assert_eq!(
            unity_samples.1[i].len(),
            as_shot_samples.1[i].len(),
            "frame {i}: different eligible sample populations"
        );
        let unity_pipeline = antitone::adjustment_layers::AdjustmentLayerSnapshot::new(vec![
            antitone::adjustment_layers::AdjustmentLayer::global(unity.photos[i]),
        ])
        .and_then(|layers| {
            antitone::adjustment_layers::AdjustmentLayerPipeline::new(
                unity.input,
                unity.roll,
                layers,
                matrix,
            )
        })
        .unwrap();
        let as_shot_pipeline = antitone::adjustment_layers::AdjustmentLayerSnapshot::new(vec![
            antitone::adjustment_layers::AdjustmentLayer::global(as_shot.photos[i]),
        ])
        .and_then(|layers| {
            antitone::adjustment_layers::AdjustmentLayerPipeline::new(
                as_shot.input,
                as_shot.roll,
                layers,
                matrix,
            )
        })
        .unwrap();
        let unity_mean = antitone::sampling::mean(
            &unity_samples.1[i]
                .iter()
                .map(|&p| unity_pipeline.apply(p, [0, 0]).map(|v| v.rgb))
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
        )
        .unwrap();
        let as_shot_mean = antitone::sampling::mean(
            &as_shot_samples.1[i]
                .iter()
                .map(|&p| as_shot_pipeline.apply(p, [0, 0]).map(|v| v.rgb))
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
        )
        .unwrap();
        let uy = (AP1_TO_XYZ * unity_mean).y;
        let ay = (AP1_TO_XYZ * as_shot_mean).y;
        luminance_ratio.push((uy / ay).log2().abs());
        chroma_difference.push(
            ((unity_mean / uy) - (as_shot_mean / ay))
                .abs()
                .max_element(),
        );
    }
    luminance_ratio.sort_by(f64::total_cmp);
    chroma_difference.sort_by(f64::total_cmp);
    let q = |v: &[f64], p: f64| v[((v.len() - 1) as f64 * p) as usize];
    println!(
        "frame-mean luminance |stops| p50={:.5} p95={:.5} max={:.5}",
        q(&luminance_ratio, 0.5),
        q(&luminance_ratio, 0.95),
        q(&luminance_ratio, 1.)
    );
    println!(
        "frame-mean chroma difference p50={:.5} p95={:.5} max={:.5}",
        q(&chroma_difference, 0.5),
        q(&chroma_difference, 0.95),
        q(&chroma_difference, 1.)
    );
    // Diagnostic bounds, not photographic acceptance: a visibly different analyzed roll should fail.
    assert!(q(&luminance_ratio, 1.) < 0.02, "frame mean differs");
    assert!(q(&chroma_difference, 1.) < 0.05, "frame colour differs");
}
