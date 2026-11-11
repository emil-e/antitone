use super::*;
use crate::gpu::submission::SubmissionGate;
use crate::{
    dust_repair::{RepairOrigin, SupportMask},
    gpu::test_inputs::AuthoredPhotograph,
    project::{Development, Project, Source, SourceDevelopment, SourceId},
};
mod cancellation;
#[cfg(feature = "ocio-reference")]
mod corrected_source;
mod source_scale;
#[cfg(feature = "ocio-reference")]
mod starting_exposure;

fn fixture(scan: &Scan) -> (Project, SourceId) {
    let (width, height) = scan.dimensions().unwrap();
    let a = scan.area;
    let mut source_development = SourceDevelopment {
        decode: scan.source().metadata().development(scan.white_balance),
        dimensions: [width, height],
        active_area: [a.x, a.y, a.width, a.height],
        camera_model: scan.camera_model.clone(),
        camera_to_ap1: scan.camera_to_ap1.to_cols_array_2d(),
    };
    source_development.decode = Development::for_source(&source_development);
    let mut project = Project {
        input: crate::processing::InputSettings::Negative {
            dmin: Some(DVec3::splat(0.8)),
            dmax: Some(DVec3::splat(0.02)),
            response: crate::processing::Response::film(),
            gamma: crate::processing::DEFAULT_GAMMA,
            balance: Default::default(),
        },
        roll: RollSettings::default(),
        ..Default::default()
    };
    let id = project
        .add_source(Source {
            development_mode: crate::project::DevelopmentMode::Unity,
            id: SourceId(0),
            path: "fixture.dng".into(),
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
fn calibrated_neutral_readout_matches_an_ordinary_finite_source_sample() {
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
    )));
    let (mut project, id) = fixture(&scan);
    project.input = InputSettings::negative(crate::processing::Calibration {
        dmin: DVec3::splat(0.7),
        dmax: DVec3::splat(0.14),
    });
    if let InputSettings::Negative { gamma, .. } = &mut project.input {
        *gamma = 0.1;
    }
    project.roll.exposure = 0.5;
    let mut recipe = project.image_recipe(id).unwrap();
    recipe.neutral.correction = scan.flat_field().cloned();
    let input = recipe.neutral.input;
    let matrix = scan.positive_matrix(&input).unwrap();
    assert!(
        AuthoredPhotograph::new(
            &device.limits(),
            input,
            RollSettings::default(),
            recipe.grade.clone(),
            matrix,
            area
        )
        .is_ok(),
        "valid photographic controls must not require a parameter-wide neutral range proof"
    );
    let actual_local = crate::scan::ImageSource::new(&scan, input)
        .unwrap()
        .sample([area.x + 16, area.y + 16])
        .unwrap();
    assert!(
        actual_local.as_vec3().is_finite(),
        "this local sample exercises ordinary HDR, not arithmetic saturation"
    );

    let gate = SubmissionGate::default();
    let mut local_source = crate::gpu::repair_source::Source::from_recipe(
        &device,
        &renderer,
        (&queue, &gate),
        &scan,
        &recipe,
    )
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
}

fn repair_demand(
    scan: &Scan,
    recipe: &crate::image_recipe::ImageRecipe,
    device: &wgpu::Device,
    index: usize,
) -> Vec<crate::gpu::evaluation::CellRegion> {
    use crate::gpu::evaluation::{self as ev, Budget, CellRegion, Domain, Grid};
    let photo = crate::gpu::photograph::PhotographGraph::from_recipe(
        &device.limits(),
        ev::camera_source(scan).unwrap(),
        None,
        recipe,
    )
    .unwrap();
    let repair =
        crate::gpu::repairs::definition::healing_image(&photo.unrepaired, &recipe.repairs[index])
            .unwrap();
    let frame = crate::frame_raster::FrameRaster::full(recipe.composition).unwrap();
    let region = CellRegion::new([0, 0], frame.output);
    let root = ev::reconstruct(
        photo.processed.clone(),
        Domain {
            bounds: Some(region),
            grid: Grid::from_frame(frame),
            meaning: photo.processed.domain().meaning,
        },
    )
    .unwrap();
    root.regions_for(
        &repair,
        region,
        Budget {
            tile: [512, 128],
            scratch_bytes: 32 << 20,
        },
    )
    .unwrap()
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
    assert_eq!(recipe.repairs.len(), 2);
    assert!(
        crate::repair_source::evaluate(
            &crate::scan::ImageSource::new(&scan, recipe.neutral.input).unwrap(),
            &recipe.repairs[1]
        )
        .is_err(),
        "an irrelevant source-edge repair would actually fail if prepared"
    );
    assert!(
        !repair_demand(&scan, &recipe, &device, 0).is_empty(),
        "the common graph requires the cropped-in repair"
    );
    assert!(
        repair_demand(&scan, &recipe, &device, 1).is_empty(),
        "the common graph omits the invalid cropped-out repair"
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
        let stopped = || true;
        assert!(matches!(
            write(
                &renderer,
                &device,
                (&queue, &gate),
                job(&recipe),
                (&path, Overwrite::Confirmed),
                &mut Execution {
                    cancelled: &stopped
                },
                || Ok(())
            ),
            Err(Error::Cancelled)
        ));
        assert_eq!(std::fs::read(&path).unwrap(), b"keep existing output");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
    let mut stale_project = project.clone();
    stale_project
        .source_mut(id)
        .unwrap()
        .development
        .decode
        .white_balance[0] *= 2.;
    let stale = stale_project.image_recipe(id).unwrap();
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
    let (device, queue, renderer) = super::super::tests::context();
    assert!(
        !repair_demand(&scan, &recipe, &device, 0).is_empty(),
        "a cell at the crop edge contributes real area"
    );
    let before = project.clone();
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
