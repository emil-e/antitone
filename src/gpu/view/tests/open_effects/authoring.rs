use super::*;
use crate::{
    frame_raster::FrameRaster,
    gpu::view::{RenderedView, ViewKernels, ViewSource, ViewSourceSpec},
    pixel_pipeline::authoring_examples::{SignedCurve, TentConvolution},
};

#[test]
fn parameterized_definitions_reach_native_regional_resident_and_physical_consumers_from_one_collection()
 {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let file = crate::dng_fixture::Fixture {
        side: 192,
        bits: 16,
        ramp: true,
        marker: Some((70, 90, 60000)),
        ..Default::default()
    }
    .write();
    let scan = crate::scan::Scan::open(file.path(), None).unwrap();
    let area = scan.area;
    let pipeline = PixelPipeline {
        stages: vec![
            PixelStage::new(SignedCurve::new(1.2, 0.15).unwrap()),
            PixelStage::new(TentConvolution::new(0.8).unwrap()),
        ],
    };
    let source = crate::scan::ImageSource::new(&scan, InputSettings::Digital).unwrap();
    let scalar = crate::adjustment_layers::GradingPipeline::new(
        Default::default(),
        AdjustmentLayerSnapshot::new(vec![]).unwrap(),
    )
    .unwrap();
    let curved = (area.y..area.y + area.height)
        .flat_map(|y| (area.x..area.x + area.width).map(move |x| [x, y]))
        .map(|p| {
            let value = scalar.apply(source.sample(p).unwrap(), p).unwrap();
            value / (DVec3::ONE + value.abs() * f64::from(0.15f32)) * f64::from(1.2f32)
        })
        .collect::<Vec<_>>();
    // Independent nine-tap product weights, rather than the GPU's two convex-accumulation passes.
    let mut expected = vec![];
    for y in 0..area.height {
        for x in 0..area.width {
            let mut rgb = DVec3::ZERO;
            for (dy, wy) in [(-1, 0.25), (0, 0.5), (1, 0.25)] {
                for (dx, wx) in [(-1, 0.25), (0, 0.5), (1, 0.25)] {
                    let sx = (i64::from(x) + dx).clamp(0, i64::from(area.width - 1)) as u32;
                    let sy = (i64::from(y) + dy).clamp(0, i64::from(area.height - 1)) as u32;
                    rgb += curved[(sy * area.width + sx) as usize] * wx * wy;
                }
            }
            expected.push(rgb);
        }
    }
    assert!(
        max_error(
            &expected,
            &native(&device, &queue, &renderer, &scan, pipeline.clone())
        ) < 2e-5
    );
    let omitted = native(&device, &queue, &renderer, &scan, PixelPipeline::default());
    assert!(
        max_error(&expected, &omitted) > 0.01,
        "the accepted collection must actually change pixels"
    );
    let only_curve = native(
        &device,
        &queue,
        &renderer,
        &scan,
        PixelPipeline {
            stages: vec![PixelStage::new(SignedCurve::new(1.2, 0.15).unwrap())],
        },
    );
    assert!(
        max_error(&expected, &only_curve) > 0.01,
        "the marked cell must expose omitted convolution independently of the curve"
    );
    let camera = scan
        .tile_region(area, 0, 0, area.width, area.height)
        .unwrap();
    let processing = input(
        &device,
        area,
        scan.positive_matrix(&InputSettings::Digital).unwrap(),
    )
    .with_pixel_pipeline(pipeline.clone());
    let frame =
        FrameRaster::full(Composition::new(area, Default::default(), Default::default()).unwrap())
            .unwrap();
    let kernels = pollster::block_on(ViewKernels::new(&device)).unwrap();
    let physical = ViewSource::acquire_many(
        &device,
        &queue,
        &Default::default(),
        renderer.clone(),
        kernels,
        &scan,
        None,
        vec![
            ViewSourceSpec::new(
                7,
                frame,
                InputSettings::Digital,
                scan.positive_matrix(&InputSettings::Digital).unwrap(),
                None,
            )
            .unwrap()
            .with_pipeline(pipeline)
            .unwrap(),
        ],
        || false,
    )
    .unwrap()
    .unwrap()
    .remove(0);
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let expected = crate::rendering::CpuRenderer::new(target)
            .unwrap()
            .render(&expected)
            .unwrap()
            .into_iter()
            .map(|p| p.as_dvec3())
            .collect::<Vec<_>>();
        let regional = crate::refinement::refine(
            &renderer,
            &device,
            (&queue, &Default::default()),
            (&scan, None),
            processing.clone(),
            target,
            (&std::sync::atomic::AtomicU64::new(1), 1),
            crate::refinement::ViewRequest {
                area,
                output: [area.width, area.height],
                frame: None,
            },
        )
        .unwrap()
        .unwrap();
        assert!(
            max_error(
                &expected,
                &crate::viewer::readback::image_pixels(&device, &queue, &regional)
            ) < 2e-5,
            "regional composition"
        );
        let mut resident = pollster::block_on(crate::viewer::GpuImage::new_continuous_shared(
            &renderer,
            &device,
            area.width,
            area.height,
            &camera,
            scan.positive_matrix(&InputSettings::Digital).unwrap(),
            target,
        ))
        .unwrap();
        resident
            .set_geometry(
                SourceGeometry::original_tile(
                    [area.x, area.y, area.width, area.height],
                    [area.width, area.height],
                )
                .unwrap(),
            )
            .unwrap();
        let mut encoder = device.create_command_encoder(&Default::default());
        resident
            .prepare_processing(&device, &mut encoder, &processing)
            .unwrap();
        queue.submit([encoder.finish()]);
        assert!(
            max_error(
                &expected,
                &crate::viewer::readback::image_pixels(&device, &queue, &resident)
            ) < 2e-5,
            "resident composition"
        );
        let view = RenderedView::new(&device, physical.clone(), &processing, target, None).unwrap();
        view.ensure_submitted(
            &device,
            &queue,
            &Default::default(),
            &egui::Context::default(),
        );
        assert!(
            max_error(
                &expected,
                &crate::viewer::readback::image_pixels(&device, &queue, &view.image)
            ) < 2e-5,
            "physical composition"
        );
    }
}
