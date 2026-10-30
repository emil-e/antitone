use super::*;
use crate::{
    dust_repair::{RepairOrigin, SupportMask},
    project::{Development, Project, Source, SourceDevelopment, SourceId},
};
use std::cell::Cell;
#[cfg(feature = "ocio-reference")]
mod corrected_source;
mod source_scale;
#[cfg(feature = "ocio-reference")]
mod starting_exposure;

fn fixture(scan: &Scan) -> (Project, SourceId) {
    let (width, height) = scan.dimensions().unwrap();
    let a = scan.area;
    let mut source_development = SourceDevelopment {
        decode: Development::new("test-backend".into(), [1.; 4]),
        lens_correction: scan.source().metadata().lens_identity().unwrap(),
        dng_development: scan.source().metadata().dng_identity(),
        dimensions: [width, height],
        active_area: [a.x, a.y, a.width, a.height],
        camera_model: scan.camera_model.clone(),
        camera_to_ap1: scan.camera_to_ap1.to_cols_array_2d(),
        input_floor: scan.source().input_floor(),
    };
    source_development.decode = Development::for_source(&source_development, [1.; 4]);
    let mut project = Project {
        input: crate::processing::InputSettings::Negative {
            dmin: Some(DVec3::splat(0.8)),
            dmax: Some(DVec3::splat(0.02)),
            input_floor: crate::processing::CAMERA_INPUT_FLOOR,
            response: crate::processing::Response::film(),
            gamma: crate::processing::DEFAULT_GAMMA,
            balance: Default::default(),
        },
        roll: RollSettings::default(),
        ..Default::default()
    };
    let id = project
        .add_source(Source {
            id: SourceId(0),
            path: "fixture.dng".into(),
            reference_only: false,
            development: source_development,

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
            adjustment_layers: Some(crate::project::AdjustmentLayers::with_global(
                Default::default(),
            )),
        })
        .unwrap();
    (project, id)
}

#[test]
fn a_repair_free_output_plan_does_not_require_an_unrepresentable_neutral_image() {
    let (device, queue, renderer) = super::super::tests::context();
    let file = crate::raw::fixture::bayer_dng();
    let mut scan = Scan::open(file.path(), None).unwrap();
    let area = scan.area;
    scan.set_flat_field(Some(Arc::new(
        crate::flat_field::FlatField::from_nodes(
            [area.x, area.y, area.width, area.height],
            [2, 2],
            vec![DVec3::splat(0.065535); 4],
        )
        .unwrap(),
    )))
    .unwrap();
    let (mut project, id) = fixture(&scan);
    project.input = InputSettings::negative(crate::processing::Calibration {
        dmin: DVec3::splat(0.7),
        dmax: DVec3::splat(0.14),
        input_floor: scan.source().input_floor(),
    });
    if let InputSettings::Negative { gamma, .. } = &mut project.input {
        *gamma = 0.1;
    }
    project.roll.exposure = -16.;
    let mut recipe = project.image_recipe(id).unwrap();
    recipe.neutral.correction = scan.flat_field().cloned();
    let input = recipe.neutral.processing_input().unwrap();
    let matrix = scan.positive_matrix(&input).unwrap();
    assert!(
        ProcessingInput::new(
            &device,
            input,
            RollSettings::default(),
            recipe.grade.clone(),
            matrix,
            area
        )
        .is_err(),
        "the fixture must require folded exposure for global GPU admission"
    );
    ProcessingInput::new(
        &device,
        input,
        recipe.roll,
        recipe.grade.clone(),
        matrix,
        area,
    )
    .unwrap();
    let actual_local = crate::scan::ImageSource::new(&scan, input)
        .unwrap()
        .sample([area.x + 16, area.y + 16])
        .unwrap();
    assert!(
        actual_local.as_vec3().is_finite(),
        "the local repair value is representable even though the parameter-wide neutral bound is not"
    );

    let gate = SubmissionGate::default();
    let mut local_source =
        crate::gpu::repair_source::Source::new(&device, &renderer, (&queue, &gate), &scan, input)
            .unwrap();
    let local = crate::dust_repair::BaseImage::read(
        &mut local_source,
        crate::geometry::SourceRegion::from([area.x + 16, area.y + 16, 1, 1]),
        None,
        &|| false,
    )
    .unwrap()
    .unwrap();
    for channel in 0..3 {
        assert!((f64::from(local[0][channel]) / actual_local[channel] - 1.).abs() < 2e-5);
    }

    let stages = recipe.pixel_pipeline().unwrap();
    let plan = crate::acquisition::Plan::new(crate::acquisition::Request {
        output: FrameRaster::full(recipe.composition).unwrap(),
        stages: &stages,
        sampling: crate::acquisition::Sampling::Originals {
            reconstruction: crate::acquisition::SourceSupport([0; 2]),
        },
    })
    .unwrap();
    let repairs = crate::gpu::repair_source::resolve_plan(
        &device,
        &renderer,
        (&queue, &gate),
        &scan,
        &recipe,
        &plan,
        &mut Execution::default(),
    )
    .unwrap()
    .unwrap();
    assert!(repairs.is_empty());
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
            [-32, 0],
        )
        .unwrap();
    let bits = vec![true; 16 * 16];
    project
        .source_mut(id)
        .unwrap()
        .repairs
        .insert(
            SupportMask::new([a.x, a.y], [16, 16], bits).unwrap(),
            RepairOrigin::Manual,
            [64, 0],
        )
        .unwrap();
    let recipe = project.image_recipe(id).unwrap();
    assert_eq!(recipe.repairs().len(), 2);
    assert!(
        crate::repair_source::evaluate(
            &crate::scan::ImageSource::new(&scan, recipe.neutral.processing_input().unwrap())
                .unwrap(),
            &recipe.repairs()[1]
        )
        .is_err(),
        "an irrelevant source-edge repair would actually fail if prepared"
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
            [32, 0],
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
