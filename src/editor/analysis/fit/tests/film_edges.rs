use super::*;
use crate::spatial_metadata::{Boundary, CommonSize, Group, Metadata, Region, SourceRegions};

fn attach_film(request: &mut Request, index: usize, boundaries: [Boundary; 4]) {
    let id = request.document.project.order[index];
    let sources: Vec<_> = request
        .document
        .spatial_inputs()
        .into_iter()
        .map(|input| SourceRegions {
            film: (input.source == id).then(|| Region {
                angle: 0.,
                skew: 0.,
                boundaries: boundaries.clone(),
            }),
            input,
            observation: None,
            picture: None,
            holder_opening: None,
            material: None,
        })
        .collect();
    let metadata = Metadata {
        groups: vec![Group {
            development: request.document.project.sources[0]
                .development
                .decode
                .clone(),
            code_scale: sources[0].input.code_scale,
            common_size: CommonSize::fit(&sources),
            sources,
        }],
        ..Default::default()
    };
    metadata.validate().unwrap();
    request.document.spatial_metadata = Arc::new(metadata.clone());
    assert_eq!(
        request.document.ready_spatial_metadata(),
        metadata,
        "the fixture must actually supply fresh film support"
    );
}

fn left_edge(x: f64) -> [Boundary; 4] {
    [
        Boundary::Measured { plane: [1., 0., x] },
        Boundary::Unknown,
        Boundary::Unknown,
        Boundary::Unknown,
    ]
}

#[test]
fn inset_checks_the_complete_pixel_footprint_on_both_axis_and_oblique_edges() {
    let f = |x, y| rect([x, y, 8, 8]);
    assert!(
        !inside_film(f(40, 40), &[[1., 0., 42.]], 2.),
        "a centre inside film cannot admit a footprint crossing its edge"
    );
    assert!(
        !inside_film(f(43, 40), &[[1., 0., 42.]], 2.),
        "fully on film is insufficient inside the exclusion band"
    );
    assert!(inside_film(f(44, 40), &[[1., 0., 42.]], 2.));
    assert!(!inside_film(f(51, 40), &[[-1., 0., -60.]], 2.));
    assert!(inside_film(f(50, 40), &[[-1., 0., -60.]], 2.));
    // Unit (3/5, 4/5) normal: centre (44, 54) has distance 3.6 from the line,
    // but the top-left footprint corner has distance -1.3, outside the band.
    assert!(!inside_film(f(40, 50), &[[0.6, 0.8, 66.]], 2.));
    assert!(inside_film(f(44, 52), &[[0.6, 0.8, 66.]], 2.));
    assert!(!inside_film(f(27, 50), &[[0.6, -0.8, -30.]], 2.));
    assert!(inside_film(f(40, 50), &[[0.6, -0.8, -30.]], 2.));
    assert!(
        inside_film(f(0, 0), &[], 100.),
        "absence of a measured film edge must not invent an inset from the camera bounds"
    );
}

#[test]
fn measured_film_edge_filters_both_populations_without_rephasing_or_dropping_the_photo() {
    for reviewed in [true, false] {
        let (mut request, _directory, device) = request();
        if reviewed {
            approved(&mut request);
        } else {
            request.film = Some(rect([8, 8, 496, 496]));
            request.picture = Some(rect([96, 96, 320, 320]));
        }
        // Declared film-support hypothesis, not a claim that this ramp fixture contains
        // a physical cut. Picture metadata is deliberately absent: full-frame abstention
        // must not discard an independently available film limit.
        attach_film(&mut request, 0, left_edge(96.));
        let before = request.document.project.clone();
        let physical = request.document.spatial_metadata.clone();
        let (scheduler, _) = observed();
        let services = Services {
            exposures: Arc::default(),
            sources: scheduler.provider(None),
            device,
        };
        let captured = input(&services, &request);
        let mut state = Fit::new(&services.device, request.clone(), captured.clone()).unwrap();
        state.geometry().unwrap();
        for index in 0..2 {
            let mut scan = futures_lite::future::block_on(services.acquire(
                Arc::new(request.clone()),
                index,
                Cancellation::default(),
            ))
            .unwrap()
            .unwrap();
            let baseline_film = scan
                .analysis_samples(scan.area, MAX_SAMPLES_PER_FRAME, || false)
                .unwrap()
                .unwrap();
            assert_eq!(baseline_film.attempted, 3844);
            assert_eq!(
                baseline_film.pixels.len(),
                3844,
                "all declared camera pixels pass numerical/capture rejection; the spatial guard must add real protection"
            );
            // The 496×496 active area has a 62×62 grid with centres 12, 20, ..., 500.
            // Full 8×8 support plus the two-source-pixel inset from x96 keeps x108 onward.
            // Independently enumerate original camera pixels, not a second mask/grid helper.
            let mut expected_film = vec![];
            let mut expected_picture = vec![];
            for y in (12..504).step_by(8) {
                for x in (12..504).step_by(8) {
                    if index == 0 && x < 108 {
                        continue;
                    }
                    let p = scan.pixel(x, y).unwrap();
                    expected_film.push(p);
                    if (100..416).contains(&x) && (100..416).contains(&y) {
                        expected_picture.push(p);
                    }
                }
            }
            assert!(state.sample(index, scan, || false).unwrap());
            let frame = &state.owned[index];
            let expected = if index == 0 {
                [3100, 1560]
            } else {
                [3844, 1600]
            };
            assert_eq!([frame.film.attempted, frame.picture.attempted], expected);
            for (samples, pixels) in [
                (&frame.film, expected_film),
                (&frame.picture, expected_picture),
            ] {
                assert_eq!(samples.pixels.len(), pixels.len());
                for (i, (actual, expected)) in samples.pixels.iter().zip(pixels).enumerate() {
                    assert_eq!(
                        *actual, expected,
                        "surviving sample {i} retains its original source position"
                    );
                }
            }
            assert_eq!(
                [
                    frame.film.rejected_capture,
                    frame.film.rejected_development,
                    frame.film.rejected_floor,
                    frame.picture.rejected_capture,
                    frame.picture.rejected_development,
                    frame.picture.rejected_floor
                ],
                [0; 6]
            );
            eprintln!(
                "film_edge_trial reviewed={reviewed} source_index={index} film_before=3844 film_after={} picture_before=1600 picture_after={} numeric_rejected=0",
                frame.film.pixels.len(),
                frame.picture.pixels.len()
            );
        }
        let outcome = state.finish(&services.device, || false).unwrap().unwrap();
        assert_eq!(
            outcome.frames, 2,
            "the edge source remains a contributing photograph"
        );
        assert!(
            outcome
                .document
                .project
                .dmin()
                .unwrap()
                .iter()
                .all(|v| (v - 3000. / 4031.).abs() <= 2. / 65535.),
            "useful clean film base must survive the additional exclusion"
        );
        assert_eq!(
            outcome
                .document
                .project
                .sources
                .iter()
                .map(|s| s.crop)
                .collect::<Vec<_>>(),
            before.sources.iter().map(|s| s.crop).collect::<Vec<_>>()
        );
        assert_eq!(
            *outcome.document.spatial_metadata, *physical,
            "sampling policy must not move descriptive boundaries/common dimensions"
        );
        assert_eq!(request.document.project, before);
        assert_eq!(*request.document.spatial_metadata, *physical);

        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn nominal_crop_and_authored_areas_do_not_admit_light_into_either_analysis_population() {
    let (mut request, _directory, device) = request();
    approved(&mut request);
    for row in &mut request.areas.as_mut().unwrap().entries {
        row.areas
            .edit_picture(Coverage::rectangle(rect([8, 8, 496, 496])));
    }
    attach_film(&mut request, 0, std::array::from_fn(|_| Boundary::Unknown));
    let metadata = Arc::make_mut(&mut request.document.spatial_metadata);
    for (i, row) in metadata.groups[0].sources.iter_mut().enumerate() {
        let picture = Region {
            angle: 0.,
            skew: 0.,
            boundaries: [
                [1., 0., 96.],
                [-1., 0., -416.],
                [0., 1., 96.],
                [0., -1., -416.],
            ]
            .map(|plane| Boundary::Measured { plane }),
        };
        row.picture = Some(picture.clone());
        row.observation = Some(picture);
        if i == 0 {
            row.observation.as_mut().unwrap().boundaries[0] = Boundary::Unknown;
            row.picture.as_mut().unwrap().boundaries[0] = Boundary::Inferred {
                plane: [1., 0., 96.],
            };
            row.material = Some(crate::scan_regions::Material {
                size: [496, 496],
                xs: (0..62).map(|i| 4 + 8 * i).collect(),
                ys: (0..62).map(|i| 4 + 8 * i).collect(),
                labels: (0..62)
                    .flat_map(|_| (0..62).map(|x| if x < 11 { 'L' } else { 'F' }))
                    .collect(),
            });
        }
    }
    metadata.groups[0].common_size = CommonSize::fit(&metadata.groups[0].sources);
    metadata.validate().unwrap();
    let crop = crate::auto_crop::propose_spatial(&metadata.groups[0].sources[0]).unwrap();
    assert_eq!(
        crop.region([8, 8, 496, 496].into()).unwrap(),
        rect([96, 96, 320, 320]),
        "crop retains the nominal frame, including the missing-film portion"
    );
    let before = request.document.project.clone();
    let physical = request.document.spatial_metadata.clone();
    let (scheduler, _) = observed();
    let services = Services {
        exposures: Arc::default(),
        sources: scheduler.provider(None),
        device,
    };
    let captured = input(&services, &request);
    let mut state = Fit::new(&services.device, request.clone(), captured).unwrap();
    state.geometry().unwrap();
    for index in 0..2 {
        let mut scan = futures_lite::future::block_on(services.acquire(
            Arc::new(request.clone()),
            index,
            Cancellation::default(),
        ))
        .unwrap()
        .unwrap();
        assert_eq!(
            scan.analysis_samples(scan.area, MAX_SAMPLES_PER_FRAME, || false)
                .unwrap()
                .unwrap()
                .pixels
                .len(),
            3844
        );
        let mut film = vec![];
        let mut picture = vec![];
        for y in (12..504).step_by(8) {
            for x in (12..504).step_by(8) {
                if index == 0 && x < 108 {
                    continue;
                }
                let p = scan.pixel(x, y).unwrap();
                film.push(p);
                if (100..416).contains(&x) && (100..416).contains(&y) {
                    picture.push(p);
                }
            }
        }
        assert!(state.sample(index, scan, || false).unwrap());
        let expected = if index == 0 {
            [3100, 1560]
        } else {
            [3844, 1600]
        };
        assert_eq!(
            [
                state.owned[index].film.pixels.len(),
                state.owned[index].picture.pixels.len()
            ],
            expected
        );
        assert_eq!([film.len(), picture.len()], expected);
        for (actual, expected) in [
            (&state.owned[index].film.pixels, film),
            (&state.owned[index].picture.pixels, picture),
        ] {
            for (i, (a, b)) in actual.iter().zip(expected).enumerate() {
                assert_eq!(*a, b, "source sample {i}");
            }
        }
    }
    let outcome = state.finish(&services.device, || false).unwrap().unwrap();
    assert_eq!(outcome.frames, 2);
    assert!(
        outcome
            .document
            .project
            .dmin()
            .unwrap()
            .iter()
            .all(|v| (v - 3000. / 4031.).abs() <= 2. / 65535.),
        "valid film base outside the picture still supplies Dmin"
    );
    assert_eq!(*outcome.document.spatial_metadata, *physical);
    assert_eq!(request.document.project, before);
    scheduler.close_and_wait().unwrap();
}

#[test]
fn shared_holder_support_filters_authored_analysis_without_trimming_nominal_crop() {
    let (mut request, _directory, device) = request();
    approved(&mut request);
    attach_film(&mut request, 0, std::array::from_fn(|_| Boundary::Unknown));
    let metadata = Arc::make_mut(&mut request.document.spatial_metadata);
    let row = &mut metadata.groups[0].sources[0];
    row.picture = Some(Region {
        angle: 0.,
        skew: 0.,
        boundaries: [
            [1., 0., 96.],
            [-1., 0., -416.],
            [0., 1., 96.],
            [0., -1., -416.],
        ]
        .map(|plane| Boundary::Measured { plane }),
    });
    row.holder_opening = Some(Region {
        angle: 0.,
        skew: 0.,
        boundaries: [
            Boundary::Unknown,
            Boundary::Unknown,
            Boundary::Unknown,
            Boundary::Inferred {
                plane: [0., -1., -312.],
            },
        ],
    });
    assert_eq!(
        crate::auto_crop::propose_spatial(row)
            .unwrap()
            .region([8, 8, 496, 496].into())
            .unwrap(),
        rect([96, 96, 320, 320])
    );
    let before = request.document.project.clone();
    let (scheduler, _) = observed();
    let services = Services {
        exposures: Arc::default(),
        sources: scheduler.provider(None),
        device,
    };
    let captured = input(&services, &request);
    let mut state = Fit::new(&services.device, request.clone(), captured).unwrap();
    state.geometry().unwrap();
    let scan = futures_lite::future::block_on(services.acquire(
        Arc::new(request.clone()),
        0,
        Cancellation::default(),
    ))
    .unwrap()
    .unwrap();
    assert!(state.sample(0, scan, || false).unwrap());
    // Source centres12..300retain37film rows; picture centres100..300retain26.
    assert_eq!(state.owned[0].film.pixels.len(), 2294);
    assert_eq!(state.owned[0].picture.pixels.len(), 1040);
    assert_eq!(request.document.project, before);
    scheduler.close_and_wait().unwrap();
}

#[test]
fn insufficient_roll_film_support_is_not_recovered_by_relaxing_the_edge_band() {
    let (mut request, _directory, device) = request();
    approved(&mut request);
    attach_film(
        &mut request,
        0,
        [
            Boundary::Measured {
                plane: [1., 0., 192.],
            },
            Boundary::Measured {
                plane: [-1., 0., -320.],
            },
            Boundary::Unknown,
            Boundary::Unknown,
        ],
    );
    let metadata = Arc::make_mut(&mut request.document.spatial_metadata);
    metadata.groups[0].sources[1].film = metadata.groups[0].sources[0].film.clone();
    let before = request.document.project.clone();
    let (scheduler, _) = observed();
    let services = Services {
        exposures: Arc::default(),
        sources: scheduler.provider(None),
        device,
    };
    let captured = input(&services, &request);
    let mut state = Fit::new(&services.device, request.clone(), captured).unwrap();
    state.geometry().unwrap();
    for index in 0..2 {
        let scan = futures_lite::future::block_on(services.acquire(
            Arc::new(request.clone()),
            index,
            Cancellation::default(),
        ))
        .unwrap()
        .unwrap();
        assert!(state.sample(index, scan, || false).unwrap());
        // Fourteen retained grid columns leave enough picture statistics, but
        // their 868 film samples per photo cannot satisfy the 2048 roll minimum.
        assert_eq!(state.owned[index].picture.pixels.len(), 560);
        assert_eq!(state.owned[index].film.pixels.len(), 868);
    }
    let error = state.finish(&services.device, || false).err().unwrap();
    assert!(
        error
            .to_string()
            .contains("too few eligible roll film samples for Dmin"),
        "{error}"
    );
    assert_eq!(request.document.project, before);
    scheduler.close_and_wait().unwrap();
}

#[test]
fn unknown_camera_and_inferred_limits_do_not_exclude_samples() {
    let (mut request, _directory, device) = request();
    approved(&mut request);
    attach_film(
        &mut request,
        0,
        [
            Boundary::Inferred {
                plane: [1., 0., 500.],
            },
            Boundary::CameraClipped,
            Boundary::Unknown,
            Boundary::Unknown,
        ],
    );
    let (scheduler, _) = observed();
    let services = Services {
        exposures: Arc::default(),
        sources: scheduler.provider(None),
        device,
    };
    let captured = input(&services, &request);
    let mut state = Fit::new(&services.device, request.clone(), captured).unwrap();
    state.geometry().unwrap();
    let scan = futures_lite::future::block_on(services.acquire(
        Arc::new(request),
        0,
        Cancellation::default(),
    ))
    .unwrap()
    .unwrap();
    assert!(state.sample(0, scan, || false).unwrap());
    assert_eq!(state.owned[0].film.pixels.len(), 3844);
    assert_eq!(state.owned[0].picture.pixels.len(), 1600);
    scheduler.close_and_wait().unwrap();
}

#[test]
fn too_little_picture_after_edge_exclusion_fails_without_relaxing_or_installing_partial_grade() {
    let (mut request, _directory, device) = request();
    approved(&mut request);
    attach_film(&mut request, 1, left_edge(400.));
    let before = request.document.project.clone();
    let (scheduler, _) = observed();
    let services = Services {
        exposures: Arc::default(),
        sources: scheduler.provider(None),
        device,
    };
    let captured = input(&services, &request);
    let error = futures_lite::future::block_on(run(
        request.clone(),
        captured,
        services,
        Cancellation::default(),
        |_, _| {},
    ))
    .err()
    .unwrap();
    assert!(
        error
            .to_string()
            .contains("only 40 of 40 picture samples are eligible; need 512"),
        "{error}"
    );
    assert_eq!(request.document.project, before);
    scheduler.close_and_wait().unwrap();
}
