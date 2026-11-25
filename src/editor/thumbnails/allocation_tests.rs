use super::*;
use crate::acquisition::camera::{ProcessingSource, SourceGeometry};
use crate::evaluation::{BufferImage, ImageDelivery};
use crate::photographic::graph::request::PhotographRequest;
use crate::photographic::graph::view_source::SourceRequest;
use crate::viewer::view::{RenderedView, ViewImage, ViewKernels};

#[test]
fn completed_thumbnails_charge_only_the_allocations_they_still_own() {
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
        let (device, queue, renderer) = crate::photographic::test_support::render_context();
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
        let capture = PhotographRequest::Recipe(recipe.clone());
        let spec = capture
            .plan_sampling(
                SourceRequest::for_frame(frame).unwrap(),
                crate::acquisition::Sampling::CameraCells {
                    region: area,
                    size: frame.output,
                },
            )
            .unwrap();
        let (definition, spec) = capture
            .sampled_output(
                &device.limits(),
                &camera,
                None,
                spec,
                renderer.looks(),
                renderer.drt(OutputTarget::Srgb),
            )
            .unwrap();
        let context = renderer.evaluation();
        let evaluation = ViewImage::evaluation(context, definition.clone()).unwrap();
        crate::evaluation::test_support::take_evaluation_buffer_creations();
        let recorded = if produce {
            context.produce_image_on_worker(evaluation, &queue).unwrap()
        } else {
            let ImageDelivery::Drawable(mut images) =
                context.request_images(vec![evaluation]).unwrap()
            else {
                panic!("control: exercise deferred recording");
            };
            images.remove(0)
        };
        let BufferImage::Produced(output) = &recorded.pixels else {
            panic!("captured endpoint");
        };
        let final_bytes = output.as_buffer().unwrap().size() + camera.input.size();
        let dependencies = output.image.used_inputs();
        if produce {
            let [_, allocated_bytes] =
                crate::evaluation::test_support::take_evaluation_buffer_creations();
            assert!(
                allocated_bytes > 0,
                "control: the worker allocated evaluation scratch"
            );
        } else {
            assert!(
                !dependencies.is_empty(),
                "control: recording owns GPU dependencies"
            );
        }
        let region = definition.domain().finite().unwrap();
        let image = ViewImage::from_recorded(
            renderer.evaluation().clone(),
            kernels,
            spec,
            definition,
            region,
            recorded,
        )
        .unwrap();
        if produce {
            assert_eq!(image.byte_len(), final_bytes);
        } else {
            assert!(
                image.byte_len() > final_bytes,
                "recording owns pending work"
            );
        }
        let view =
            RenderedView::from_image(&device, image.clone(), capture, OutputTarget::Srgb).unwrap();
        view.ensure_submitted(&device, &queue, &egui::Context::default());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert_eq!(image.byte_len(), final_bytes);
        assert!(dependencies.iter().all(|input| input.upgrade().is_none()));
        assert_eq!(renderer.evaluation().retained_bytes(), 0);
    }
}
