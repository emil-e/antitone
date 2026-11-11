use super::*;
use crate::gpu::submission::SubmissionGate;
use crate::processing::InputSettings;
use glam::{DMat3, DVec3};

fn recipe(scan: &Scan, input: InputSettings) -> crate::image_recipe::ImageRecipe {
    let source = crate::source::source_metadata(
        &scan.source().file.path,
        &scan.source().metadata(),
        scan.source().white_balance(),
    )
    .unwrap();
    let mut project = crate::project::Project {
        input,
        ..Default::default()
    };
    let id = project.add_source(source).unwrap();
    let mut recipe = project.image_recipe(id).unwrap();
    recipe.neutral.correction = scan.flat_field().cloned();
    recipe.neutral.reference = scan.reference_exposure.clone();
    recipe
}

#[test]
fn repair_inputs_share_corrected_signed_hdr_and_negative_values_across_blocks_and_cache_policies() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let gate = SubmissionGate::default();
    let no_cache =
        evaluation::Context::new(&device, evaluation::CachePolicy { generated_bytes: 0 });
    let file = crate::raw::fixture::bayer_dng_with_camera(
        [260, 1000, 600],
        glam::DMat3::IDENTITY.to_cols_array(),
    );
    let mut scan = Scan::open(file.path(), None).unwrap();
    let area = scan.area;
    let field = Arc::new(
        crate::flat_field::FlatField::from_nodes(
            [area.x, area.y, area.width, area.height],
            [2, 2],
            vec![
                DVec3::new(0.9, 1.1, 0.8),
                DVec3::new(1.2, 0.9, 1.1),
                DVec3::new(1., 1.2, 0.9),
                DVec3::new(0.8, 1., 1.2),
            ],
        )
        .unwrap(),
    );
    scan.set_flat_field(Some(field));
    let region = SourceRegion::from([area.x + 11, area.y + 13, 17, 9]);
    let needed: Vec<_> = (0..region.width * region.height)
        .map(|index| index % 3 != 0)
        .collect();

    for (input, ev) in [
        (InputSettings::Digital, 0.),
        (InputSettings::Digital, 8.),
        (
            crate::processing::Calibration {
                dmin: DVec3::splat(0.8),
                dmax: DVec3::splat(0.02),
            }
            .into(),
            0.,
        ),
    ] {
        scan.reference_exposure = Some(crate::exposure_reference::ReferenceExposure::Matched(
            crate::exposure_reference::StartingExposure {
                ev,
                samples: 1024,
                spread: 0.,
            },
        ));
        let reference = crate::scan::ImageSource::new(&scan, input).unwrap();
        let mut uncorrected_scan =
            Scan::with_reference(scan.source().clone(), scan.reference_exposure.clone());
        uncorrected_scan.set_flat_field(None);
        let uncorrected = crate::scan::ImageSource::new(&uncorrected_scan, input).unwrap();
        let recipe = recipe(&scan, input);
        let mut source =
            Source::from_recipe(&device, &renderer, (&queue, &gate), &scan, &recipe).unwrap();
        source.budget.tile = [3, 2];
        let pixels = source
            .read(region, Some(&needed), &|| false)
            .unwrap()
            .unwrap();
        assert_eq!(pixels.len(), needed.len());
        let mut correction_difference: f64 = 0.;
        let mut signed = false;
        let mut hdr = false;

        for (index, pixel) in pixels.iter().enumerate() {
            if !needed[index] {
                assert_eq!(*pixel, [0.; 3]);
                continue;
            }
            let point = [
                region.x + index as u32 % region.width,
                region.y + index as u32 / region.width,
            ];
            let expected = reference.sample(point).unwrap();
            correction_difference = correction_difference.max(
                (expected - uncorrected.sample(point).unwrap())
                    .abs()
                    .max_element(),
            );
            signed |= expected.min_element() < 0.;
            hdr |= expected.max_element() > 1.;
            for channel in 0..3 {
                assert!(
                    (f64::from(pixel[channel]) - expected[channel]).abs()
                        < 2e-5 * expected.abs().max_element().max(1.),
                    "{input:?} {point:?}: {pixel:?} != {expected:?}"
                );
            }
        }
        assert!(
            correction_difference > 1e-4,
            "the fixture must distinguish correction omission"
        );
        if input == InputSettings::Digital {
            assert!(
                signed,
                "the camera profile must exercise signed working light"
            );
            if ev != 0. {
                assert!(hdr, "the captured exposure must exercise HDR light");
            }
        }

        source.budget.tile = [5, 3];
        assert_eq!(
            source
                .read(region, Some(&needed), &|| false)
                .unwrap()
                .unwrap(),
            pixels
        );
        source.context = &no_cache;
        assert_eq!(
            source
                .read(region, Some(&needed), &|| false)
                .unwrap()
                .unwrap(),
            pixels
        );
        assert!(source.read(region, None, &|| true).unwrap().is_none());
        assert_eq!(
            source
                .read(region, Some(&needed), &|| false)
                .unwrap()
                .unwrap(),
            pixels
        );
    }
    assert_eq!(no_cache.retained_bytes(), 0);
}

#[test]
fn repair_inputs_keep_masked_values_zero_and_saturate_extreme_required_values() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let file = crate::raw::fixture::bayer_dng_with_rgb([0; 3]);
    let scan = Scan::open(file.path(), None).unwrap();
    let mut input = InputSettings::negative(crate::processing::Calibration {
        dmin: DVec3::splat(0.7),
        dmax: DVec3::splat(0.14),
    });
    if let InputSettings::Negative { gamma, .. } = &mut input {
        *gamma = 0.1;
    }
    let region = SourceRegion::from([scan.area.x + 16, scan.area.y + 16, 2, 2]);
    // Model 6 is finite at zero light. This declared camera conversion makes the
    // complete neutral calculation exceed f32 while its shader remains bounded.
    let matrix = DMat3::from_diagonal(DVec3::splat(1e33));
    let scalar = crate::processing::InputPipeline::new(input, matrix).unwrap();
    assert!(scalar.apply(DVec3::ZERO).unwrap().rgb.min_element() > f64::from(f32::MAX));
    let graph = crate::gpu::photograph::PhotographGraph::original(
        &device.limits(),
        crate::gpu::evaluation::camera_source(&scan).unwrap(),
        None,
        crate::gpu::interpretation::InputTransform::new(input, matrix).unwrap(),
        crate::gpu::grading::Grading::new(
            &device.limits(),
            Default::default(),
            crate::adjustment_layers::AdjustmentLayerSnapshot::new(vec![]).unwrap(),
            scan.area,
        )
        .unwrap(),
        vec![],
        &Default::default(),
    )
    .unwrap();
    let gate = SubmissionGate::default();
    let mut source = Source::from_image(&renderer, (&queue, &gate), graph.neutral).unwrap();
    assert_eq!(
        source
            .read(region, Some(&[false; 4]), &|| false)
            .unwrap()
            .unwrap(),
        vec![[0.; 3]; 4]
    );
    let required = source.read(region, None, &|| false).unwrap().unwrap();
    assert_eq!(required.len(), 4);
    assert!(
        required
            .iter()
            .flatten()
            .all(|value| value.is_finite() && *value > 1e20),
        "extreme required positives remain finite HDR, rather than rejecting the repair"
    );
    assert_eq!(
        source
            .read(region, Some(&[false; 4]), &|| false)
            .unwrap()
            .unwrap(),
        vec![[0.; 3]; 4]
    );
}

#[test]
fn masked_nonfinite_authored_values_are_unused_but_required_values_reject() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let file = crate::raw::fixture::bayer_dng_with_rgb([0; 3]);
    let scan = Scan::open(file.path(), None).unwrap();
    let input = InputSettings::negative(crate::processing::Calibration {
        dmin: DVec3::splat(0.7),
        dmax: DVec3::splat(0.14),
    });
    let gate = SubmissionGate::default();
    let mut source = Source::from_recipe(
        &device,
        &renderer,
        (&queue, &gate),
        &scan,
        &recipe(&scan, input),
    )
    .unwrap();
    source.image = crate::gpu::evaluation::pointwise(
        source.image.clone(),
        crate::gpu::evaluation::Meaning::Neutral,
        crate::gpu::evaluation::Function::new(
            "fn overflow(p:vec4f, position:vec2f)->vec4f{return vec4f(p.rgb*1e35,p.w);}",
            "overflow",
        ),
    );
    let region = SourceRegion::from([scan.area.x + 16, scan.area.y + 16, 2, 2]);
    assert_eq!(
        source
            .read(region, Some(&[false; 4]), &|| false)
            .unwrap()
            .unwrap(),
        vec![[0.; 3]; 4]
    );
    assert!(matches!(
        source.read(region, Some(&[true, false, false, false]), &|| false),
        Err(Error::NumericalOverflow)
    ));
    assert_eq!(
        source
            .read(region, Some(&[false; 4]), &|| false)
            .unwrap()
            .unwrap(),
        vec![[0.; 3]; 4]
    );
}

#[test]
fn repair_inputs_accept_developed_zero_and_native_white() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let file = crate::raw::fixture::bayer_dng_with_rgb([0, 65535, 600]);
    let scan = Scan::open(file.path(), None).unwrap();
    let region = SourceRegion::from([scan.area.x + 16, scan.area.y + 16, 8, 8]);
    let camera = scan.pixel(region.x, region.y).unwrap();
    assert_eq!(camera.x, 0.);
    assert_eq!(camera.y, 1.);
    let gate = SubmissionGate::default();
    let recipe = recipe(&scan, InputSettings::Digital);
    let mut source =
        Source::from_recipe(&device, &renderer, (&queue, &gate), &scan, &recipe).unwrap();
    let pixels = source.read(region, None, &|| false).unwrap().unwrap();
    assert_eq!(pixels.len(), 64);
    assert!(pixels.iter().flatten().all(|value| value.is_finite()));
}

#[test]
fn dust_search_reads_original_unrepaired_neutral_not_repaired_or_graded_output() {
    use crate::dust_repair::{Repair, RepairId, RepairOrigin, SupportMask};
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let gate = SubmissionGate::default();
    let file = crate::dng_fixture::Fixture {
        side: 192,
        bits: 16,
        ramp: true,
        marker: Some((70, 90, 60000)),
        ..Default::default()
    }
    .write();
    let scan = Scan::open(file.path(), None).unwrap();
    let mut recipe = recipe(&scan, InputSettings::Digital);
    recipe.neutral.reference = Some(crate::exposure_reference::ReferenceExposure::Matched(
        crate::exposure_reference::StartingExposure {
            ev: 2.,
            samples: 1024,
            spread: 0.,
        },
    ));
    recipe.roll.exposure = 1.;
    let mut layers = recipe.grade.layers().to_vec();
    layers[0].photo.exposure = 1.;
    recipe.grade = crate::adjustment_layers::AdjustmentLayerSnapshot::new(layers).unwrap();
    recipe.repairs = vec![Repair {
        id: RepairId(1),
        enabled: true,
        origin: RepairOrigin::Manual,
        support: SupportMask::new([66, 86], [9, 9], vec![true; 81]).unwrap(),
        offset: [40, 0],
    }];
    let (scan, mut recipe) = crate::photo_preparation::native_image(&scan, &recipe).unwrap();
    let region = SourceRegion::from([66, 86, 9, 9]);
    let graph = crate::gpu::photograph::RecipeGraph::originals(
        evaluation::camera_source(&scan).unwrap(),
        None,
        &recipe,
    )
    .unwrap();
    let original = graph.interpreted().unwrap().unrepaired;
    let photograph = graph.photograph(&device.limits()).unwrap();
    let read = |image| {
        Source::from_image(&renderer, (&queue, &gate), image)
            .unwrap()
            .read(region, None, &|| false)
            .unwrap()
            .unwrap()
    };
    let mut search =
        Source::from_recipe(&device, &renderer, (&queue, &gate), &scan, &recipe).unwrap();
    assert_eq!(
        search.image().domain().meaning,
        evaluation::Meaning::Neutral
    );
    assert_eq!(
        search.image().domain().grid,
        evaluation::Grid::axis_aligned([0.; 2], [1.; 2])
    );
    assert!(search.image().same_definition(&original));
    let searched = search.read(region, None, &|| false).unwrap().unwrap();
    assert_eq!(searched, read(original));
    let repaired = read(photograph.neutral);
    let graded = read(photograph.processed);
    let different = |a: &Vec<[f32; 3]>, b: &Vec<[f32; 3]>| {
        a.iter()
            .zip(b)
            .any(|(a, b)| (0..3).any(|c| (a[c] - b[c]).abs() > 1e-4))
    };
    assert!(
        different(&searched, &repaired),
        "the defect fixture must distinguish the repaired tap"
    );
    assert!(
        different(&repaired, &graded),
        "the grade must distinguish the downstream tap"
    );

    // A previously accepted but unsolvable repair cannot prevent searching its unrepaired input.
    recipe.repairs[0].support =
        SupportMask::new([scan.area.x, scan.area.y], [3, 3], vec![true; 9]).unwrap();
    recipe.repairs[0].offset = [30, 0];
    let edge = SourceRegion::from([scan.area.x, scan.area.y, 3, 3]);
    let values = Source::from_recipe(&device, &renderer, (&queue, &gate), &scan, &recipe)
        .unwrap()
        .read(edge, None, &|| false)
        .unwrap()
        .unwrap();
    assert_eq!(values.len(), 9);
    assert!(values.iter().flatten().all(|value| value.is_finite()));
    let repaired = crate::gpu::photograph::RecipeGraph::originals(
        evaluation::camera_source(&scan).unwrap(),
        None,
        &recipe,
    )
    .unwrap()
    .photograph(&device.limits())
    .unwrap()
    .neutral;
    let error = Source::from_image(&renderer, (&queue, &gate), repaired)
        .unwrap()
        .read(edge, None, &|| false)
        .unwrap_err();
    assert!(error.to_string().contains("two-pixel exterior"), "{error}");
}
