use super::*;
use crate::gpu::evaluation as e;
use crate::gpu::input::ProcessingInput;
use crate::{frame_raster::FrameRaster, geometry::Composition};
use std::sync::atomic::{AtomicUsize, Ordering};

struct ConstantSamples {
    domain: e::Domain,
    pixel: [f32; 4],
    reads: Arc<AtomicUsize>,
}
impl e::Source for ConstantSamples {
    fn domain(&self) -> e::Domain {
        self.domain
    }
    fn identity(&self) -> Option<e::RetainedKey> {
        Some(e::RetainedKey::new((
            "constant source samples",
            self.domain,
            self.pixel.map(f32::to_bits),
        )))
    }
    fn read(&self, region: e::CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        Ok(vec![self.pixel; (region.width * region.height) as usize])
    }
}

fn frame() -> FrameRaster {
    FrameRaster::full(
        Composition::new([0, 0, 4, 3].into(), Default::default(), Default::default()).unwrap(),
    )
    .unwrap()
}

fn input(limits: &wgpu::Limits, exposure: f64) -> ProcessingInput {
    ProcessingInput::new(
        limits,
        crate::processing::InputSettings::Digital,
        crate::processing::RollSettings {
            exposure,
            ..Default::default()
        },
        crate::adjustment_layers::AdjustmentLayerSnapshot::new(Vec::new()).unwrap(),
        DMat3::IDENTITY,
        frame().composition.area(),
    )
    .unwrap()
}

fn samples(reads: Arc<AtomicUsize>) -> Arc<ConstantSamples> {
    Arc::new(ConstantSamples {
        domain: e::Domain {
            bounds: Some([0, 0, 4, 3].into()),
            grid: e::Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: e::Meaning::CameraCodes,
        },
        // Original camera-code white is 65535, including continuous f32 storage.
        pixel: [-0.25, 0.125, 0.5, 0.].map(|value| value * 65535.),
        reads,
    })
}

#[test]
fn preview_can_choose_an_existing_finer_grid_but_rejects_insufficient_or_different_coverage() {
    let limits = wgpu::Limits::default();
    let reads = Arc::new(AtomicUsize::new(0));
    let source = e::source(samples(reads.clone()));
    let input = input(&limits, 1.);
    let (default, _) = crate::gpu::photograph::PhotographGraph::preview_with_input(
        &limits,
        source.clone(),
        None,
        &input,
        frame(),
        None,
    )
    .unwrap();
    let grid = FrameRaster {
        output: [8, 6],
        ..frame()
    };
    let (finer, plan) = crate::gpu::photograph::PhotographGraph::preview_with_input(
        &limits,
        source.clone(),
        None,
        &input,
        frame(),
        Some(grid),
    )
    .unwrap();
    assert_eq!(plan.output(), grid);
    assert_eq!(finer.processed.domain(), default.processed.domain());
    assert_ne!(finer.neutral.identity(), default.neutral.identity());
    assert_ne!(finer.processed.identity(), default.processed.identity());
    for invalid in [
        FrameRaster {
            output: [2, 2],
            ..grid
        },
        FrameRaster {
            min: crate::geometry::FramePoint([1., 0.]),
            ..grid
        },
    ] {
        assert!(
            crate::gpu::photograph::PhotographGraph::preview_with_input(
                &limits,
                source.clone(),
                None,
                &input,
                frame(),
                Some(invalid),
            )
            .is_err()
        );
    }
    assert_eq!(reads.load(Ordering::Relaxed), 0);
}

#[test]
fn original_to_processed_definition_requires_no_device_or_source_read() {
    let limits = wgpu::Limits::default();
    let reads = Arc::new(AtomicUsize::new(0));
    let samples = samples(reads.clone());
    let camera = e::source(samples.clone());
    let photographs = [1., 2.].map(|exposure| {
        crate::gpu::photograph::PhotographGraph::preview_with_input(
            &limits,
            camera.clone(),
            None,
            &input(&limits, exposure),
            frame(),
            None,
        )
        .unwrap()
        .0
    });
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    assert!(
        photographs
            .iter()
            .all(|photo| photo.processed.identity().is_some())
    );
    assert_eq!(
        photographs[0].neutral.identity(),
        photographs[1].neutral.identity()
    );
    assert_ne!(
        photographs[0].processed.identity(),
        photographs[1].processed.identity()
    );
    assert_eq!(
        photographs[0].processed.domain().finite().unwrap(),
        [0, 0, 4, 3].into()
    );
    e::Source::read(samples.as_ref(), [0, 0, 1, 1].into()).unwrap();
    assert_eq!(reads.load(Ordering::Relaxed), 1);
}

#[test]
fn native_view_regions_follow_the_graphs_neighbourhood_contracts() {
    let limits = wgpu::Limits::default();
    let reads = Arc::new(AtomicUsize::new(0));
    let source = e::source(samples(reads.clone()));
    for (sigma, expected) in [
        (0., e::CellRegion::new([0, 0], [4, 3])),
        (1., e::CellRegion::new([-4, -4], [12, 11])),
    ] {
        let mut input = input(&limits, 2.);
        if sigma > 0. {
            input = input.with_pixel_pipeline(crate::pixel_pipeline::PixelPipeline {
                stages: vec![crate::pixel_pipeline::PixelStage::new(
                    crate::image_mtf::ImageMtfParameters { sigma },
                )],
            });
        }
        let (photo, _) = crate::gpu::photograph::PhotographGraph::preview_with_input(
            &limits,
            source.clone(),
            None,
            &input,
            frame(),
            None,
        )
        .unwrap();
        assert_eq!(
            photo
                .processed
                .region_for(&photo.neutral, photo.processed.domain().finite().unwrap())
                .unwrap(),
            Some(expected),
        );
        let equivalent_source = e::source(samples(Arc::new(AtomicUsize::new(0))));
        let mut different = samples(Arc::new(AtomicUsize::new(0)));
        Arc::get_mut(&mut different).unwrap().pixel[0] = 0.;
        let different_source = e::source(different);
        assert_eq!(
            photo
                .neutral
                .region_for(&photo.processed, expected)
                .unwrap(),
            None
        );
        assert_eq!(
            photo
                .processed
                .region_for(
                    &equivalent_source,
                    photo.processed.domain().finite().unwrap()
                )
                .unwrap(),
            Some([0, 0, 4, 3].into())
        );
        assert_eq!(
            photo
                .processed
                .region_for(
                    &different_source,
                    photo.processed.domain().finite().unwrap()
                )
                .unwrap(),
            None
        );
    }
    assert_eq!(reads.load(Ordering::Relaxed), 0);
}

#[test]
fn complete_graph_precedes_io_and_uses_its_live_neutral_value_without_another_source_read() {
    pollster::block_on(async {
        let (device, queue, _) = crate::gpu::test_support::render_context();
        let context = e::Context::new(&device, e::CachePolicy { generated_bytes: 0 });
        let scope = context.materializations();
        let reads = Arc::new(AtomicUsize::new(0));
        let (photograph, _) = crate::gpu::photograph::PhotographGraph::preview_with_input(
            &device.limits(),
            e::source(samples(reads.clone())),
            None,
            &input(&device.limits(), 2.),
            frame(),
            None,
        )
        .unwrap();
        let complete = photograph.processed;
        assert_eq!(reads.load(Ordering::Relaxed), 0);
        assert!(complete.identity().is_some());
        let gate = crate::gpu::submission::SubmissionGate::default();
        let request = |root: Arc<e::ImageNode>| {
            let grid = root.domain().grid;
            context
                .request(
                    root,
                    e::Region {
                        bounds: [0, 0, 4, 3].into(),
                        grid,
                    },
                    e::Budget {
                        tile: [4, 3],
                        scratch_bytes: 4096,
                    },
                )
                .unwrap()
        };
        let native = context
            .materialize(
                &queue,
                &gate,
                &scope,
                request(photograph.neutral),
                e::ImageFormat::Rgba32Float,
                e::ImageBacking::Buffer,
            )
            .unwrap()
            .await
            .unwrap();
        let first_reads = reads.load(Ordering::Relaxed);
        assert!(
            first_reads > 0,
            "the control must execute the supplied source"
        );
        let output = context
            .materialize(
                &queue,
                &gate,
                &scope,
                request(complete.clone()),
                e::ImageFormat::Rgba32Float,
                e::ImageBacking::Buffer,
            )
            .unwrap()
            .await
            .unwrap();
        assert_eq!(
            reads.load(Ordering::Relaxed),
            first_reads,
            "the complete downstream graph must consume its live materialized upstream value"
        );
        let pixels = e::read_buffer(&device, &queue, output.as_buffer().unwrap());
        assert_eq!(pixels.len(), 12);
        for pixel in &pixels {
            for (actual, expected) in pixel[..3].iter().zip([-1., 0.5, 2.]) {
                assert!((actual - expected).abs() < 1e-6, "{pixel:?}");
            }
            assert_eq!(pixel[3], 8.);
        }
        assert_eq!(context.retained_bytes(), 0);
        scope.clear();
        let rebuilt = context
            .materialize(
                &queue,
                &gate,
                &scope,
                request(complete),
                e::ImageFormat::Rgba32Float,
                e::ImageBacking::Buffer,
            )
            .unwrap()
            .await
            .unwrap();
        assert!(
            reads.load(Ordering::Relaxed) > first_reads,
            "removing discovery must expose real upstream execution rather than a no-op read counter"
        );
        assert_eq!(
            pixels,
            e::read_buffer(&device, &queue, rebuilt.as_buffer().unwrap())
        );
        drop(native);
    });
}
