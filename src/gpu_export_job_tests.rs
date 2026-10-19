use super::*;
use crate::{
    dust_repair::{DonorChoice, RepairOrigin, SupportMask},
    project::{Development, Project, Source, SourceDevelopment, SourceId},
};
use std::cell::Cell;
#[cfg(feature = "ocio-reference")]
#[path = "gpu_export_corrected_source_tests.rs"]
mod corrected_source;

fn fixture(scan: &Scan) -> (Project, SourceId) {
    let (width, height) = scan.dimensions().unwrap();
    let a = scan.area;
    let source_development = SourceDevelopment {
        lens_correction: scan.source().metadata().lens_identity().unwrap(),
        dng_development: scan.source().metadata().dng_identity(),
        dimensions: [width, height],
        active_area: [a.x, a.y, a.width, a.height],
        camera_model: scan.camera_model.clone(),
        camera_to_ap1: scan.camera_to_ap1.to_cols_array_2d(),
        input_floor: scan.source().input_floor(),
    };
    let mut project = Project {
        development: Some(Development::for_source(&source_development, scan.fixed_wb)),
        dmin: Some([0.8; 3]),
        dmax: Some([0.02; 3]),
        input_floor: Some(scan.input_floor),
        roll: RollSettings::new_grade(),
        ..Default::default()
    };
    let id = project
        .add_source(Source {
            id: SourceId(0),
            path: "fixture.dng".into(),
            reference_only: false,
            development: source_development,
            photo: Default::default(),
            orientation: Default::default(),
            crop: Some(crate::crop::Crop {
                center: [
                    f64::from(a.x) + f64::from(a.width) / 2.,
                    f64::from(a.y) + f64::from(a.height) / 2.,
                ],
                size: [
                    a.width - if a.width > 128 { 96 } else { 16 },
                    a.height - if a.height > 128 { 96 } else { 16 },
                ],
                angle: 0.13,
            }),
            repairs: Default::default(),
            local_adjustments: None,
        })
        .unwrap();
    (project, id)
}

#[test]
fn immutable_worker_job_resolves_cold_repairs_skips_irrelevant_failures_and_honours_stops() {
    let (device, queue, renderer) = super::super::tests::context();
    let gate = SubmissionGate::default();
    let scan = Scan::open(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        None,
    )
    .unwrap();
    let a = scan.area;
    let (mut project, id) = fixture(&scan);
    project
        .source_mut(id)
        .unwrap()
        .repairs
        .insert(
            SupportMask::new(
                [a.x + a.width / 2, a.y + a.height / 2],
                [3, 3],
                vec![true; 9],
            )
            .unwrap(),
            RepairOrigin::Manual,
            DonorChoice::Offset([-32, 0]),
        )
        .unwrap();
    let bits = (0..a.width * a.height)
        .map(|i| {
            let x = i % a.width;
            let y = i / a.width;
            x >= 8
                && y >= 8
                && x + 8 < a.width
                && y + 8 < a.height
                && (x == 8 || y == 8 || x + 9 == a.width || y + 9 == a.height)
        })
        .collect();
    project
        .source_mut(id)
        .unwrap()
        .repairs
        .insert(
            SupportMask::new([a.x, a.y], [a.width, a.height], bits).unwrap(),
            RepairOrigin::Manual,
            DonorChoice::Automatic,
        )
        .unwrap();
    let recipe = project.image_recipe(id).unwrap();
    assert_eq!(recipe.repairs().len(), 2);
    assert!(
        crate::repair_source::evaluate(
            &scan,
            recipe.neutral.processing_calibration().unwrap(),
            recipe.roll,
            &recipe.repairs()[1]
        )
        .is_err(),
        "an irrelevant repair would actually fail if prepared"
    );
    assert_eq!(
        crate::repair_source::resolve_output(&scan, &recipe)
            .unwrap()
            .len(),
        1
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("result.tif");
    let job = |recipe| Job {
        scan: &scan,
        field: None,
        recipe,
        format: Format::Tiff(OutputTarget::Srgb),
    };
    let before = project.clone();
    {
        let (result, pixels) = crate::export_timing::pixels::capture(|| {
            write(
                &renderer,
                &device,
                (&queue, &gate),
                job(&recipe),
                (&path, Overwrite::Refuse),
                &mut Execution::default(),
                || Ok(()),
            )
        });
        result.unwrap();
        let codes = pixels.unwrap().codes;
        assert!(
            codes.iter().any(|pixel| pixel != &codes[0]),
            "cold repaired fixture must retain rendered variation"
        );
        assert_eq!(project, before);
    }
    std::fs::write(&path, b"keep existing output").unwrap();
    {
        let calls = Cell::new(0);
        let cancelled = || {
            let n = calls.get();
            calls.set(n + 1);
            n >= 1
        };
        assert!(
            write(
                &renderer,
                &device,
                (&queue, &gate),
                job(&recipe),
                (&path, Overwrite::Confirmed),
                &mut Execution {
                    screener: None,
                    cancelled: &cancelled
                },
                || Ok(())
            )
            .is_err()
        );
        assert!(
            calls.get() >= 2,
            "stop during cold repair preparation, after initial job admission"
        );
        let mut checks = 0;
        assert!(
            write(
                &renderer,
                &device,
                (&queue, &gate),
                job(&recipe),
                (&path, Overwrite::Confirmed),
                &mut Execution::default(),
                || {
                    checks += 1;
                    if checks > 4 {
                        Err(Error::Export("mid-job stop".into()))
                    } else {
                        Ok(())
                    }
                }
            )
            .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"keep existing output");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
    let mut stale = recipe.clone();
    stale.neutral.source_development.input_floor *= 2.;
    assert!(
        write(
            &renderer,
            &device,
            (&queue, &gate),
            job(&stale),
            (&path, Overwrite::Confirmed),
            &mut Execution::default(),
            || Ok(())
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"keep existing output");
}

#[test]
fn cold_frame_edge_repairs_use_the_common_engine_for_tiff_and_exr() {
    let scan = Scan::open(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        None,
    )
    .unwrap();
    let (mut project, id) = fixture(&scan);
    let composition = project.image_recipe(id).unwrap().composition;
    let p = composition
        .frame_to_source(crate::geometry::FramePoint([
            0.,
            f64::from(composition.size()[1]) / 2.,
        ]))
        .0
        .map(|v| v.floor() as u32);
    project
        .source_mut(id)
        .unwrap()
        .repairs
        .insert(
            SupportMask::new(p, [1, 1], vec![true]).unwrap(),
            RepairOrigin::Manual,
            DonorChoice::Offset([32, 0]),
        )
        .unwrap();
    let recipe = project.image_recipe(id).unwrap();
    assert_eq!(
        crate::repair_source::resolve_output(&scan, &recipe)
            .unwrap()
            .len(),
        1
    );
    let before = project.clone();
    let (device, queue, renderer) = super::super::tests::context();
    let gate = SubmissionGate::default();
    let directory = tempfile::tempdir().unwrap();
    for (name, format) in [
        ("edge.tif", Format::Tiff(OutputTarget::Srgb)),
        ("edge.exr", Format::Exr(ExrPrecision::Full)),
    ] {
        write(
            &renderer,
            &device,
            (&queue, &gate),
            Job {
                scan: &scan,
                field: None,
                recipe: &recipe,
                format,
            },
            (&directory.path().join(name), Overwrite::Refuse),
            &mut Execution::default(),
            || Ok(()),
        )
        .unwrap();
        assert!(directory.path().join(name).is_file());
    }
    assert_eq!(project, before);
}
