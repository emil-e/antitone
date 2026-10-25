//! Original-footprint, scalar-mask and encoded-lifetime regressions on production objects.
use super::*;
use crate::{crop::Crop, geometry::Composition};

fn cropped(area: SourceRegion, region: SourceRegion, output: [u32; 2], angle: f64) -> FrameRaster {
    let composition = Composition::with_crop(
        area,
        Default::default(),
        Default::default(),
        Some(Crop {
            center: [
                region.x as f64 + region.width as f64 * 0.5,
                region.y as f64 + region.height as f64 * 0.5,
            ],
            size: [region.width, region.height],
            angle,
        }),
    )
    .unwrap();
    FrameRaster::new(
        composition,
        FramePoint([0., 0.]),
        FramePoint(composition.size().map(f64::from)),
        output,
    )
    .unwrap()
}

fn close(expected: DVec3, actual: [f32; 4]) {
    for c in 0..3 {
        assert!(
            actual[c].is_finite()
                && (expected[c] - actual[c] as f64).abs() <= 2e-5 + 2e-4 * expected[c].abs(),
            "{expected:?} vs {actual:?}"
        );
    }
}

#[test]
fn physical_view_colour_matches_scalar_continuous_source_masks() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let area = SourceRegion {
            x: 11,
            y: 17,
            width: 63,
            height: 47,
        };
        let frame = cropped(
            area,
            SourceRegion {
                x: 20,
                y: 24,
                width: 37,
                height: 29,
            },
            [17, 13],
            0.23,
        );
        let v = [-0.01, 0., 0.001, 0.0078125, 0.18, 1., 4., 16.];
        let values = (0..17 * 13)
            .map(|i| {
                [
                    v[i % 8],
                    v[(i * 3 + 1) % 8],
                    v[(i * 5 + 2) % 8],
                    (8 + i % 8) as f32,
                ]
            })
            .collect::<Vec<_>>();
        let source = authored_source(&device, renderer, kernels, &values, frame);
        let snapshot = PhotoGradeSnapshot::new(PhotoSettings::default(), local_pair(area)).unwrap();
        let scalar = crate::local_adjustments::LocalGrade::new(snapshot.locals().to_vec()).unwrap();
        let view = render(
            &device,
            &queue,
            source.clone(),
            snapshot,
            OutputTarget::Srgb,
        );
        let actual = read(&device, &queue, &view.graded.as_ref().unwrap().pixels);
        for (i, (input, result)) in values.iter().zip(actual).enumerate() {
            let expected = scalar
                .apply(
                    crate::processing::InvertedPixel {
                        rgb: DVec3::new(input[0] as f64, input[1] as f64, input[2] as f64),
                        floored: std::array::from_fn(|c| (input[3] as u32 >> c) & 1 != 0),
                    },
                    frame.source_point([(i % 17) as f64 + 0.5, (i / 17) as f64 + 0.5]),
                )
                .unwrap();
            close(expected.rgb, result);
            assert_eq!(result[3], input[3]);
        }
        assert_eq!(read(&device, &queue, &source.pixels), values);
    });
}

fn field(area: SourceRegion) -> Arc<crate::flat_field::FlatField> {
    Arc::new(
        crate::flat_field::FlatField::from_nodes(
            [area.x, area.y, area.width, area.height],
            [2, 2],
            vec![
                DVec3::new(1.2, 0.9, 1.1),
                DVec3::new(0.95, 1.3, 1.05),
                DVec3::new(1.1, 1.05, 0.88),
                DVec3::new(1.25, 0.98, 1.15),
            ],
        )
        .unwrap(),
    )
}

#[test]
fn physical_view_original_footprints_match_scalar_correction_signed_hdr_and_floor_provenance() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let mut scan = scan();
        let area = scan.area;
        let correction = field(area);
        scan.set_flat_field(Some(correction.clone())).unwrap();
        let mut anchors = calibration();
        // A narrow density span makes the fixture's dark patches HDR, without clipping.
        anchors.dmax = anchors.dmin * 0.8;
        let cal = correction.processing_calibration(anchors).unwrap();
        let roll = RollSettings {
            gamma: 0.65,
            ..RollSettings::new_grade()
        };
        let region = SourceRegion {
            x: area.x + area.width / 8,
            y: area.y + area.height / 8,
            width: area.width * 3 / 4,
            height: area.height * 3 / 4,
        };
        let frame = cropped(area, region, [13, 9], 0.23);
        let spec = ViewSourceSpec::new(
            1,
            frame,
            cal,
            roll,
            scan.camera_to_ap1,
            Some(correction.clone()),
        )
        .unwrap();
        let source = ViewSource::acquire(
            &device,
            &queue,
            &SubmissionGate::default(),
            renderer,
            kernels,
            &scan,
            Some(Arc::new(GpuFlatField::new(&device, correction).unwrap())),
            spec,
            || false,
        )
        .unwrap()
        .unwrap();
        let scalar = crate::local_adjustments::SpatialPipeline::new(
            cal,
            roll,
            PhotoGradeSnapshot::new(PhotoSettings::default(), vec![]).unwrap(),
            scan.camera_to_ap1,
        )
        .unwrap();
        let actual = read(&device, &queue, &source.pixels);
        for (i, result) in actual.iter().enumerate() {
            let footprint = frame.footprint([(i % 13) as u32, (i / 13) as u32]).unwrap();
            let mut expected = DVec3::ZERO;
            let mut flags = 0;
            for (p, w) in footprint.pixels(area).unwrap() {
                let value = scalar.apply(scan.pixel(p[0], p[1]).unwrap(), p).unwrap();
                expected += value.rgb * w;
                flags |= 8;
                for c in 0..3 {
                    flags |= u32::from(value.floored[c]) << c;
                }
            }
            close(expected / footprint.area(), *result);
            assert_eq!(result[3] as u32, flags);
        }
        assert!(actual.iter().any(|p| p[..3].iter().any(|v| *v < 0.)));
        assert!(actual.iter().any(|p| p[..3].iter().any(|v| *v > 1.)));
    });
}

fn raw_cases(region: SourceRegion) -> Vec<(&'static str, PhotoGradeSnapshot)> {
    let simple = |photo| PhotoGradeSnapshot::new(photo, vec![]).unwrap();
    let centre = [
        region.x as f64 + region.width as f64 * 0.5,
        region.y as f64 + region.height as f64 * 0.5,
    ];
    let locals = vec![
        LocalAdjustment {
            mask: GradientMask::Linear {
                start: SourcePoint([centre[0] - region.width as f64 * 0.2, centre[1]]),
                end: SourcePoint([centre[0] + region.width as f64 * 0.2, centre[1]]),
            },
            photo: PhotoSettings {
                exposure: 1.,
                wb: Balance {
                    warmth: 0.02,
                    tint: -0.01,
                },
                ..Default::default()
            },
            enabled: true,
            invert: false,
            strength: 0.7,
        },
        LocalAdjustment {
            mask: GradientMask::Radial {
                center: SourcePoint(centre),
                radii: [region.width as f64 * 0.25, region.height as f64 * 0.3],
                angle: 0.3,
                feather: 0.,
            },
            photo: PhotoSettings {
                exposure: -0.5,
                saturation: 1.1,
                ..Default::default()
            },
            enabled: true,
            invert: false,
            strength: 0.6,
        },
    ];
    let mut feathered = locals.clone();
    if let GradientMask::Radial { feather, .. } = &mut feathered[1].mask {
        *feather = 0.5;
    }
    vec![
        ("identity", simple(PhotoSettings::default())),
        (
            "exposure-wb",
            simple(PhotoSettings {
                exposure: 2.,
                wb: Balance {
                    warmth: 0.05,
                    tint: -0.03,
                },
                ..Default::default()
            }),
        ),
        (
            "contrast-saturation",
            simple(PhotoSettings {
                contrast: 1.25,
                contrast_colour_mix: 0.25,
                saturation: 1.25,
                saturation_density: 0.4,
                ..Default::default()
            }),
        ),
        (
            "local-feathered",
            PhotoGradeSnapshot::new(photo(), feathered).unwrap(),
        ),
        (
            "local-hard-boundary",
            PhotoGradeSnapshot::new(photo(), locals).unwrap(),
        ),
    ]
}

#[test]
fn physical_view_encoded_versions_survive_owner_release_and_abandoned_encoders_do_not_initialize() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let values = vec![[0.25, 0.5, 1., 8.]; 512 * 256];
        let area = SourceRegion {
            x: 11,
            y: 17,
            width: 63,
            height: 47,
        };
        let composition = Composition::new(area, Default::default(), Default::default()).unwrap();
        let frame = FrameRaster::new(
            composition,
            FramePoint([0., 0.]),
            FramePoint(composition.size().map(f64::from)),
            [512, 256],
        )
        .unwrap();
        let source = authored_source(&device, renderer, kernels, &values, frame);
        let photo = PhotoSettings {
            exposure: 1.,
            ..Default::default()
        };
        let snapshot = PhotoGradeSnapshot::new(photo, vec![]).unwrap();
        let roll = RollSettings {
            gamma: 0.65,
            ..RollSettings::new_grade()
        };
        let input = ProcessingInput::from(
            ProcessingParameters::positive(calibration(), roll, photo, source.spec.matrix).unwrap(),
        );
        let view = RenderedView::new(
            &device,
            source.clone(),
            calibration(),
            roll,
            snapshot,
            &input,
            OutputTarget::Srgb,
            None,
        )
        .unwrap();
        let mut abandoned = device.create_command_encoder(&Default::default());
        let graded = view.graded.as_ref().unwrap();
        source.kernels.grade(&device, &mut abandoned, graded);
        drop(abandoned);
        assert!(!*graded.submitted.lock().unwrap());
        view.ensure_submitted(
            &device,
            &queue,
            &SubmissionGate::default(),
            &egui::Context::default(),
        );
        let expected = read(&device, &queue, &graded.pixels);
        assert!(expected.iter().all(|p| *p == [0.5, 1., 2., 8.]));
        let mut deferred = device.create_command_encoder(&Default::default());
        source.kernels.grade(&device, &mut deferred, graded);
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: graded.pixels.size(),
            usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        deferred.copy_buffer_to_buffer(&graded.pixels, 0, &output, 0, graded.pixels.size());
        let owner = Arc::downgrade(&view);
        let source_owner = Arc::downgrade(&source);
        drop(view);
        drop(source);
        assert!(owner.upgrade().is_none() && source_owner.upgrade().is_none());
        queue.submit([deferred.finish()]);
        assert_eq!(read(&device, &queue, &output), expected);
    });
}
