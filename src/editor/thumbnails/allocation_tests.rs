use super::*;
use crate::gpu::evaluation::{BufferImage, RecordedImage};
use crate::gpu::photograph::request::PhotographRequest;
use crate::gpu::source::{ProcessingSource, SourceGeometry};
use crate::gpu::view::{RenderedView, ViewImage, ViewKernels, request::SourceRequest};

#[test]
fn complete_thumbnail_payload_charges_leased_dependencies_for_recording_and_worker_production() {
    let mut document = tests::fixture();
    document.project.set_dmin(Some([0.8; 3]));
    document.project.set_dmax(Some([0.02; 3]));
    document.project.output.grain.enabled = true;
    document.project.output.grain.amount = 0.;
    document.project.output.halation.enabled = true;
    let recipe = Arc::new(
        document
            .project
            .image_recipe(document.project.order[0])
            .unwrap(),
    );
    let area: crate::raw::Rect = recipe.neutral.source_development.active_area().into();
    let size = [128, 96];
    for produce in [false, true] {
        let (device, queue, renderer) = crate::gpu::test_support::render_context();
        renderer
            .evaluation()
            .materializations()
            .set_retention_enabled(false);
        let kernels = pollster::block_on(ViewKernels::new(&device)).unwrap();
        let camera = ProcessingSource::new_continuous(
            &device,
            &vec![[20000., 30000., 40000., 0.]; size[0] * size[1]],
            glam::DMat3::from_cols_array_2d(
                &recipe
                    .neutral
                    .source_development
                    .resolved()
                    .unwrap()
                    .camera_to_ap1,
            ),
            Some(
                SourceGeometry::new(
                    [area.x, area.y, area.width, area.height],
                    size.map(|v| v as u32),
                )
                .unwrap(),
            ),
        )
        .unwrap();
        let mut frame = crate::frame_raster::FrameRaster::full(
            Composition::new(area, Default::default(), Default::default()).unwrap(),
        )
        .unwrap();
        frame.output = size.map(|v| v as u32);
        let spec = SourceRequest::for_recipe(frame, &recipe)
            .unwrap()
            .with_sampling(crate::acquisition::Sampling::CameraCells {
                region: area,
                size: frame.output,
            })
            .unwrap();
        let capture = PhotographRequest::Recipe(recipe.clone());
        let (definition, spec) = capture
            .sampled_output(&renderer, &camera, None, spec, OutputTarget::Srgb)
            .unwrap();
        let work = renderer
            .evaluation()
            .prepare_image(
                ViewImage::evaluation(renderer.evaluation(), definition.clone()).unwrap(),
            )
            .unwrap();
        let recorded = if produce {
            RecordedImage {
                pixels: renderer
                    .evaluation()
                    .produce_prepared(&queue, &renderer.submissions, work, || false)
                    .unwrap()
                    .try_into()
                    .unwrap(),
                commands: None,
            }
        } else {
            assert!(work.can_record(), "control: exercise deferred recording");
            work.record(renderer.evaluation()).unwrap()
        };
        let BufferImage::Produced(output) = &recorded.pixels else {
            panic!("captured endpoint");
        };
        assert!(
            !output.inputs.is_empty(),
            "control: the endpoint leases real dependencies"
        );
        let mut buffers = vec![output.as_buffer().unwrap().clone()];
        for input in output.inputs.iter().filter_map(|input| input.as_buffer()) {
            if !buffers.contains(input) {
                buffers.push(input.clone());
            }
        }
        if !buffers.contains(&camera.input) {
            buffers.push(camera.input.clone());
        }
        let minimum = buffers.iter().map(wgpu::Buffer::size).sum::<u64>();
        assert!(
            minimum > output.as_buffer().unwrap().size() + camera.input.size(),
            "control: dependency allocations are additional payload"
        );
        let region = definition.domain().finite().unwrap();
        let image = ViewImage::from_recorded(
            renderer.clone(),
            kernels,
            spec,
            definition,
            region,
            recorded,
        )
        .unwrap();
        let view = RenderedView::from_image(&device, image, capture, OutputTarget::Srgb).unwrap();
        assert!(
            view.photographic_bytes() >= minimum,
            "produce={produce}: {} < {minimum}",
            view.photographic_bytes()
        );
        view.ensure_submitted(
            &device,
            &queue,
            &renderer.submissions,
            &egui::Context::default(),
        );
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert!(
            view.photographic_bytes() >= minimum,
            "submission does not retire leased dependencies"
        );
        assert_eq!(renderer.evaluation().retained_bytes(), 0);
    }
}
