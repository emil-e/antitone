use crate::gpu::evaluation::read_buffer;

#[cfg(feature = "ocio-reference")]
mod authoring;

use std::sync::Arc;

fn max_error(expected: &[DVec3], actual: &[[f32; 4]]) -> f64 {
    assert_eq!(expected.len(), actual.len());
    assert!(expected.iter().all(|v| v.is_finite()));
    assert!(actual.iter().flatten().all(|v| v.is_finite()));
    expected
        .iter()
        .zip(actual)
        .map(|(e, a)| {
            (*e - DVec3::new(f64::from(a[0]), f64::from(a[1]), f64::from(a[2])))
                .abs()
                .max_element()
        })
        .fold(0., f64::max)
}

#[cfg(feature = "ocio-reference")]
use crate::gpu::SourceGeometry;
use crate::{
    adjustment_layers::AdjustmentLayerSnapshot,
    geometry::{Composition, FramePoint},
    gpu::{GpuPipeline, ProcessingInput, SubmissionGate},
    pixel_pipeline::{PixelPipeline, PixelStage, test_effects::NeighbourEffect},
    processing::InputSettings,
    rendering::{OutputReference, OutputTarget},
};
use glam::{DMat3, DVec3};
use wgpu::util::DeviceExt;

fn recipe() -> PixelPipeline {
    PixelPipeline {
        stages: vec![PixelStage::new(NeighbourEffect {
            radius: 0.8,
            gain: 1.4,
        })],
    }
}

fn input(
    device: &wgpu::Device,
    area: crate::geometry::SourceRegion,
    matrix: DMat3,
) -> ProcessingInput {
    ProcessingInput::from(Arc::new(
        GpuPipeline::new(
            device,
            InputSettings::Digital,
            Default::default(),
            AdjustmentLayerSnapshot::new(vec![]).unwrap(),
            matrix,
            area,
        )
        .unwrap(),
    ))
    .with_pixel_pipeline(recipe())
}

fn neighbour(left: DVec3, center: DVec3, right: DVec3, position: [f64; 2]) -> DVec3 {
    (left + 2. * center + right) * 0.25 * 1.4
        + DVec3::splat(position[0] * 0.000001 + position[1] * 0.000002)
}

fn native(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &crate::renderer::GpuRenderer,
    scan: &crate::scan::Scan,
    stages: PixelPipeline,
) -> Vec<[f32; 4]> {
    let area = scan.area;
    let size = [area.width, area.height];
    let mut result = vec![[0.; 4]; (size[0] * size[1]) as usize];
    crate::gpu::execution::execute(
        renderer,
        device,
        (queue, &SubmissionGate::default()),
        crate::gpu::execution::Request {
            scan,
            field: None,
            input: InputSettings::Digital,
            roll: Default::default(),
            grade: AdjustmentLayerSnapshot::new(vec![]).unwrap(),
            repairs: &[],
            composition: Composition::new(area, Default::default(), Default::default()).unwrap(),
            stages,
            output: OutputReference::SceneReferred,
            look: None,
            encoding: crate::gpu::execution::SampleEncoding::LinearFloat,
            budget: crate::gpu::execution::WorkBudget::for_device(device),
        },
        |render, complete| {
            for y in (0..size[1]).step_by(128) {
                for x in (0..size[0]).step_by(128) {
                    let extent = [128.min(size[0] - x), 128.min(size[1] - y)];
                    let crate::gpu::execution::OutputBlock::Linear(values) =
                        render([x, y], extent)?
                    else {
                        panic!("linear scene output")
                    };
                    for row in 0..extent[1] {
                        result[((y + row) * size[0] + x) as usize
                            ..((y + row) * size[0] + x + extent[0]) as usize]
                            .copy_from_slice(
                                &values
                                    [(row * extent[0]) as usize..((row + 1) * extent[0]) as usize],
                            );
                    }
                }
            }
            complete()
        },
        || Ok(()),
    )
    .unwrap();
    result
}

#[test]
fn a_supplied_effect_runs_through_native_regional_and_resident_purposes() {
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
    let source = crate::scan::ImageSource::new(&scan, InputSettings::Digital).unwrap();
    let scalar = crate::adjustment_layers::GradingPipeline::new(
        Default::default(),
        AdjustmentLayerSnapshot::new(vec![]).unwrap(),
    )
    .unwrap();
    let sample = |p| scalar.apply(source.sample(p).unwrap(), p).unwrap();
    let expected: Vec<_> = (area.y..area.y + area.height)
        .flat_map(|y| (area.x..area.x + area.width).map(move |x| [x, y]))
        .map(|[x, y]| {
            neighbour(
                sample([x.saturating_sub(1).max(area.x), y]),
                sample([x, y]),
                sample([(x + 1).min(area.x + area.width - 1), y]),
                [f64::from(x) + 0.5, f64::from(y) + 0.5],
            )
        })
        .collect();
    let modified = native(&device, &queue, &renderer, &scan, recipe());
    assert!(max_error(&expected, &modified) < 2e-5);
    let clean = native(&device, &queue, &renderer, &scan, PixelPipeline::default());
    assert!(
        max_error(&expected, &clean) > 0.05,
        "omitting the supplied effect changes the photograph"
    );
    let no_neighbours: Vec<_> = (area.y..area.y + area.height)
        .flat_map(|y| (area.x..area.x + area.width).map(move |x| [x, y]))
        .map(|[x, y]| {
            neighbour(
                sample([x, y]),
                sample([x, y]),
                sample([x, y]),
                [f64::from(x) + 0.5, f64::from(y) + 0.5],
            )
        })
        .collect();
    assert!(
        expected
            .iter()
            .zip(&no_neighbours)
            .any(|(a, b)| (*a - *b).abs().max_element() > 0.01),
        "the authored defect makes neighbourhood work observable, independently of gain"
    );
    #[cfg(feature = "ocio-reference")]
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let expected: Vec<_> = crate::rendering::CpuRenderer::new(target)
            .unwrap()
            .render(&expected)
            .unwrap()
            .into_iter()
            .map(|p| p.as_dvec3())
            .collect();
        let processing = input(
            &device,
            area,
            scan.positive_matrix(&InputSettings::Digital).unwrap(),
        );
        let regional = crate::refinement::refine(
            &renderer,
            &device,
            (&queue, &SubmissionGate::default()),
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
        let actual = crate::viewer::readback::image_pixels(&device, &queue, &regional);
        assert!(
            max_error(&expected, &actual) < 2e-5,
            "regional supplied composition"
        );
        let pixels = scan
            .tile_region(area, 0, 0, area.width, area.height)
            .unwrap();
        let mut resident = pollster::block_on(crate::viewer::GpuImage::new_continuous_shared(
            &renderer,
            &device,
            area.width,
            area.height,
            &pixels,
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
        let actual = crate::viewer::readback::image_pixels(&device, &queue, &resident);
        assert!(
            max_error(&expected, &actual) < 2e-5,
            "resident supplied composition"
        );
    }
}

#[test]
fn the_same_effect_uses_affine_source_coordinates_and_declared_support_in_physical_views() {
    use crate::{
        frame_raster::FrameRaster,
        gpu::view::{RenderedView, ViewKernels, ViewSource, ViewSourceSpec},
    };
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let kernels = pollster::block_on(ViewKernels::new(&device)).unwrap();
    let area = [100, 80, 6000, 4000].into();
    let composition = Composition::with_crop(
        area,
        Default::default(),
        Default::default(),
        Some(crate::crop::Crop {
            center: [3000., 2000.],
            size: [200, 160],
            angle: 0.37,
        }),
    )
    .unwrap();
    let frame = FrameRaster::new(
        composition,
        FramePoint([21.5, 30.25]),
        FramePoint([117.5, 102.25]),
        [48, 36],
    )
    .unwrap();
    let processing = input(&device, area, DMat3::IDENTITY);
    let spec = ViewSourceSpec::new(17, frame, InputSettings::Digital, DMat3::IDENTITY, None)
        .unwrap()
        .with_pipeline(processing.pixel_pipeline().unwrap())
        .unwrap();
    let origin = frame.source_point([0.5, 0.5]).0;
    let axis = frame.source_point([1.5, 0.5]).0;
    let dx = [axis[0] - origin[0], axis[1] - origin[1]];
    let sample = |point: [f64; 2]| {
        let x = ((point[0] - origin[0]) * dx[0] + (point[1] - origin[1]) * dx[1])
            / (dx[0] * dx[0] + dx[1] * dx[1]);
        DVec3::new(
            if x < -0.5 { 4. } else { -0.03 },
            0.2 + point[1] * 0.00002,
            1.5,
        )
    };
    let upstream = spec.input_frame().unwrap();
    let values: Vec<[f32; 4]> = (0..upstream.output[1])
        .flat_map(|y| (0..upstream.output[0]).map(move |x| [x, y]))
        .map(|[x, y]| {
            let value = sample(
                upstream
                    .source_point([f64::from(x) + 0.5, f64::from(y) + 0.5])
                    .0,
            )
            .as_vec3();
            [value.x, value.y, value.z, 8.]
        })
        .collect();
    let source = Arc::new(ViewSource {
        spec,
        renderer,
        kernels,
        pixels: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("independent affine neutral field"),
            contents: bytemuck::cast_slice(&values),
            usage: wgpu::BufferUsages::STORAGE,
        }),
    });
    let view = RenderedView::new(&device, source, &processing, OutputTarget::Srgb, None).unwrap();
    view.ensure_submitted(
        &device,
        &queue,
        &SubmissionGate::default(),
        &egui::Context::default(),
    );
    let actual = read_buffer(&device, &queue, &view.graded.as_ref().unwrap().pixels);
    let expected: Vec<_> = (0..frame.output[1])
        .flat_map(|y| (0..frame.output[0]).map(move |x| [x, y]))
        .map(|[x, y]| {
            let at = |x: f64| frame.source_point([x + 0.5, f64::from(y) + 0.5]).0;
            neighbour(
                sample(at(f64::from(x) - 1.)),
                sample(at(f64::from(x))),
                sample(at(f64::from(x) + 1.)),
                at(f64::from(x)),
            )
        })
        .collect();
    assert!(max_error(&expected, &actual) < 2e-5);
    assert!(
        expected[0].x > 1.,
        "a real neighbour outside the requested frame contributes through the declared effect support"
    );
    let equal = input(&device, area, DMat3::IDENTITY);
    assert_eq!(
        processing.signature(),
        equal.signature(),
        "independently assembled equal effect values preserve reuse identity"
    );
    assert_ne!(
        processing.signature(),
        processing
            .clone()
            .with_pixel_pipeline(PixelPipeline::default())
            .signature()
    );
}
