use super::*;
use crate::{
    adjustment_layers::{AdjustmentLayer, AdjustmentLayerSettings, AdjustmentLayerSnapshot},
    crop::Crop,
    looks::{LookAsset, LookOrigin, Lut3D},
    processing::InputSettings,
};
use std::{cell::Cell, path::Path};

fn scan() -> Scan {
    Scan::open(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        None,
    )
    .unwrap()
}

fn look() -> Arc<LookAsset> {
    Arc::new(
        LookAsset::new(
            "output reference fixture".into(),
            LookOrigin::Imported,
            Arc::new(
                Lut3D::new(
                    2,
                    [-2.; 3],
                    [2.; 3],
                    (0..8)
                        .map(|i| {
                            [
                                -1.9 + 4. * (i % 2) as f32,
                                -1.95 + 3.8 * ((i / 2) % 2) as f32,
                                -1.97 + 4. * (i / 4) as f32,
                            ]
                        })
                        .collect(),
                )
                .unwrap(),
            ),
        )
        .unwrap(),
    )
}

fn request(scan: &Scan, output: OutputReference, encoding: SampleEncoding) -> Request<'_> {
    let area = scan.area;
    Request {
        scan,
        field: None,
        input: InputSettings::Digital,
        roll: Default::default(),
        grade: AdjustmentLayerSnapshot::new(vec![AdjustmentLayer::global(
            AdjustmentLayerSettings {
                exposure: 0.7,
                contrast: 1.25,
                saturation: 1.1,
                ..Default::default()
            },
        )])
        .unwrap(),
        repairs: &[],
        composition: Composition::with_crop(
            area,
            Default::default(),
            Default::default(),
            Some(Crop {
                center: [
                    f64::from(area.x) + f64::from(area.width) / 2.,
                    f64::from(area.y) + f64::from(area.height) / 2.,
                ],
                size: [23, 17],
                angle: 0.19,
            }),
        )
        .unwrap(),
        stages: Default::default(),
        output,
        look: Some(look()),
        encoding,
        budget: WorkBudget {
            source_pixels: u64::MAX,
        },
    }
}

fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &GpuRenderer,
    request: Request<'_>,
) -> Result<OutputBlock, Error> {
    let size = request.composition.size();
    let mut output = None;
    execute(
        renderer,
        device,
        (queue, &SubmissionGate::default()),
        request,
        |render, complete| {
            output = Some(render([0; 2], size)?);
            complete()
        },
        || Ok(()),
    )?;
    Ok(output.unwrap())
}

#[cfg(feature = "ocio-reference")]
#[test]
fn scene_and_display_float_results_match_scalar_and_ocio_before_reconstruction() {
    use crate::{adjustment_layers::GradingPipeline, scan::ImageSource};
    use glam::DVec3;
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let scan = scan();
    for reference in [
        OutputReference::SceneReferred,
        OutputReference::DisplayReferred(OutputTarget::Srgb),
        OutputReference::DisplayReferred(OutputTarget::DisplayP3),
    ] {
        let request = request(&scan, reference, SampleEncoding::LinearFloat);
        let source = ImageSource::new(&scan, request.input).unwrap();
        let grading = GradingPipeline::new(request.roll, request.grade.clone()).unwrap();
        let raster = FrameRaster::full(request.composition).unwrap();
        let look = request.look.clone().unwrap();
        let cpu = reference
            .display_target()
            .map(|target| crate::rendering::CpuRenderer::new(target).unwrap());
        let OutputBlock::Linear(actual) = render(&device, &queue, &renderer, request).unwrap()
        else {
            panic!("float storage must not imply RGB16 transfer/quantization");
        };
        let mut order_difference = 0_f64;
        for y in 0..raster.output[1] {
            for x in 0..raster.output[0] {
                let cells: Vec<_> = raster
                    .footprint([x, y])
                    .unwrap()
                    .pixels(scan.area)
                    .unwrap()
                    .collect();
                let working: Vec<_> = cells
                    .iter()
                    .map(|(point, _)| {
                        grading
                            .apply(source.sample(*point).unwrap(), *point)
                            .unwrap()
                    })
                    .collect();
                let values = match &cpu {
                    Some(cpu) => cpu
                        .render(
                            &working
                                .iter()
                                .map(|rgb| look.lut().apply(*rgb).unwrap())
                                .collect::<Vec<_>>(),
                        )
                        .unwrap()
                        .into_iter()
                        .map(|rgb| rgb.as_dvec3())
                        .collect::<Vec<_>>(),
                    None => working.clone(),
                };
                let mean = |values: &[DVec3]| {
                    crate::sampling::weighted_mean_iter(
                        values
                            .iter()
                            .zip(&cells)
                            .map(|(value, (_, weight))| Ok((*value, *weight))),
                    )
                    .unwrap()
                };
                let expected = mean(&values);
                let actual = actual[(y * raster.output[0] + x) as usize];
                assert!(actual[3] > 0., "reconstruction retains valid coverage");
                for (channel, expected) in actual[..3].iter().zip(expected.to_array()) {
                    let tolerance = if cpu.is_some() {
                        2e-5
                    } else {
                        2e-5 + 2e-4 * expected.abs()
                    };
                    assert!(
                        (f64::from(*channel) - expected).abs() <= tolerance,
                        "{reference:?}, pixel ({x},{y}): {actual:?}, expected {expected}"
                    );
                }
                if let Some(cpu) = &cpu {
                    let reversed = cpu
                        .render(&[look.lut().apply(mean(&working)).unwrap()])
                        .unwrap()[0]
                        .as_dvec3();
                    order_difference =
                        order_difference.max((expected - reversed).abs().max_element());
                }
            }
        }
        if cpu.is_some() {
            assert!(
                order_difference > 2e-5,
                "fixture must distinguish display rendering before reconstruction from rendering its average"
            );
        }
    }
}

#[test]
fn display_reference_can_deliver_linear_float_or_encoded_rgb16_without_a_file_codec() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let scan = scan();
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let output = OutputReference::DisplayReferred(target);
        let OutputBlock::Linear(linear) = render(
            &device,
            &queue,
            &renderer,
            request(&scan, output, SampleEncoding::LinearFloat),
        )
        .unwrap() else {
            panic!("linear result")
        };
        let OutputBlock::Rgb16 {
            target: encoded_target,
            words,
        } = render(
            &device,
            &queue,
            &renderer,
            request(&scan, output, SampleEncoding::Rgb16),
        )
        .unwrap()
        else {
            panic!("encoded result")
        };
        assert_eq!(encoded_target, target);
        let expected = target
            .encode_u16(
                &linear
                    .iter()
                    .map(|pixel| glam::Vec3::new(pixel[0], pixel[1], pixel[2]))
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        assert_eq!(
            rgb16::decode(&words).unwrap(),
            expected,
            "sample transfer/quantization must follow the same display-linear result"
        );
        assert!(
            linear
                .iter()
                .any(
                    |pixel| (target.encode(f64::from(pixel[1])).unwrap() - f64::from(pixel[1]))
                        .abs()
                        > 0.05
                ),
            "fixture must distinguish display-linear from transfer-encoded values"
        );
    }
}

#[test]
fn scene_rgb16_is_rejected_and_source_work_is_bounded_before_reading_a_unit() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let scan = scan();
    let called = Cell::new(false);
    let invalid = request(&scan, OutputReference::SceneReferred, SampleEncoding::Rgb16);
    let result = execute(
        &renderer,
        &device,
        (&queue, &SubmissionGate::default()),
        invalid,
        |_, _| {
            called.set(true);
            Ok(())
        },
        || Ok(()),
    );
    assert!(matches!(result, Err(Error::InvalidParameter(_))));
    assert!(
        !called.get(),
        "invalid scene storage must fail before handing work to a sink"
    );
    let mut bounded = request(
        &scan,
        OutputReference::SceneReferred,
        SampleEncoding::LinearFloat,
    );
    bounded.budget.source_pixels = 1;
    assert!(matches!(
        render(&device, &queue, &renderer, bounded),
        Err(Error::InvalidParameter(
            "processing unit exceeds source work budget"
        ))
    ));
    render(
        &device,
        &queue,
        &renderer,
        request(
            &scan,
            OutputReference::SceneReferred,
            SampleEncoding::LinearFloat,
        ),
    )
    .unwrap();
}

#[test]
fn cancellation_after_submission_finishes_private_readback_without_delivering_a_block() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let scan = scan();
    let gate = SubmissionGate::default();
    let submits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = submits.clone();
    gate.observe_encoded_resources(std::sync::Arc::new(move || {
        observed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }));
    let request = request(
        &scan,
        OutputReference::SceneReferred,
        SampleEncoding::LinearFloat,
    );
    let size = request.composition.size();
    let mut delivered = false;
    let result = execute(
        &renderer,
        &device,
        (&queue, &gate),
        request,
        |render, complete| {
            render([0; 2], size)?;
            delivered = true;
            complete()
        },
        || {
            if submits.load(std::sync::atomic::Ordering::Relaxed) > 0 {
                Err(Error::Cancelled)
            } else {
                Ok(())
            }
        },
    );
    assert!(matches!(result, Err(Error::Cancelled)));
    assert!(
        !delivered,
        "a submitted but superseded block remains private"
    );
    assert_eq!(submits.load(std::sync::atomic::Ordering::Relaxed), 1);
    render(
        &device,
        &queue,
        &renderer,
        self::request(
            &scan,
            OutputReference::SceneReferred,
            SampleEncoding::LinearFloat,
        ),
    )
    .unwrap();
}

#[test]
fn oversized_destination_blocks_are_rejected_without_submission_and_do_not_poison_reuse() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let scan = scan();
    let gate = SubmissionGate::default();
    let submits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = submits.clone();
    gate.observe_encoded_resources(std::sync::Arc::new(move || {
        observed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }));
    let mut request = request(
        &scan,
        OutputReference::SceneReferred,
        SampleEncoding::LinearFloat,
    );
    request.composition =
        Composition::new(scan.area, Default::default(), Default::default()).unwrap();
    assert!(request.composition.size()[0] > TILE[0]);
    let result = execute(
        &renderer,
        &device,
        (&queue, &gate),
        request,
        |render, _| {
            render([0; 2], [TILE[0] + 1, TILE[1]])?;
            Ok(())
        },
        || Ok(()),
    );
    assert!(matches!(
        result,
        Err(Error::InvalidParameter(
            "invalid processing destination unit"
        ))
    ));
    assert_eq!(submits.load(std::sync::atomic::Ordering::Relaxed), 0);
    render(
        &device,
        &queue,
        &renderer,
        self::request(
            &scan,
            OutputReference::SceneReferred,
            SampleEncoding::LinearFloat,
        ),
    )
    .unwrap();
}

#[test]
fn completion_checks_publication_rights_and_retires_resources_after_cancelled_memory_output() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let scan = scan();
    let request = request(
        &scan,
        OutputReference::DisplayReferred(OutputTarget::Srgb),
        SampleEncoding::LinearFloat,
    );
    let size = request.composition.size();
    let retired = Cell::new(false);
    let mut accepted = false;
    let result = execute(
        &renderer,
        &device,
        (&queue, &SubmissionGate::default()),
        request,
        |render, complete| {
            render([0; 2], size)?;
            assert_eq!(
                renderer.live_look_resources(),
                1,
                "execution retains its captured look through completion"
            );
            retired.set(true);
            complete()?;
            accepted = true;
            Ok(())
        },
        || {
            if retired.get() {
                Err(Error::Cancelled)
            } else {
                Ok(())
            }
        },
    );
    assert!(matches!(result, Err(Error::Cancelled)));
    assert!(!accepted);
    assert_eq!(
        renderer.live_look_resources(),
        0,
        "cancelled execution releases its GPU look lease"
    );
}
