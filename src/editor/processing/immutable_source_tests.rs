use super::*;
use crate::gpu::evaluation::{Budget, ImageNode, Meaning, Region};
use crate::gpu::parameters::ProcessingParameters;
use std::sync::{Arc, Weak};

fn camera_cells(mut image: Arc<ImageNode>) -> Arc<ImageNode> {
    while image.domain().meaning != Meaning::CameraCodes {
        image = image.input().expect("camera calculation").clone();
    }
    image
}

fn pixels(
    renderer: &crate::renderer::GpuRenderer,
    queue: &wgpu::Queue,
    image: Arc<ImageNode>,
) -> Vec<[f32; 4]> {
    let domain = image.domain();
    let request = renderer
        .evaluation()
        .request(
            image,
            Region {
                bounds: domain.finite().unwrap(),
                grid: domain.grid,
            },
            Budget {
                tile: [7, 5],
                scratch_bytes: 1 << 20,
            },
        )
        .unwrap();
    let mut pixels = Vec::new();
    renderer
        .evaluation()
        .read(
            queue,
            &Default::default(),
            &request,
            || false,
            |_, block| {
                pixels.extend_from_slice(block);
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    pixels
}

#[test]
fn project_corrections_share_fixed_camera_samples_and_retain_only_their_own_resources() {
    pollster::block_on(async {
        let (editor, device, queue) = super::tests::fixture().await;
        let renderer = crate::renderer::GpuRenderer::new(&device, &queue)
            .await
            .unwrap();
        let proxy = &editor.document.visit.canvas.ready.as_ref().unwrap().proxy;
        let image = proxy.image(OutputTarget::Srgb).unwrap();
        let mut project = editor.document.state.project.clone();
        let id = editor.document.visit.current_source.unwrap();
        let source = project.source(id).unwrap().clone();
        let mut photographs = Vec::new();
        let mut inspections = Vec::new();
        let mut fields: Vec<Weak<crate::gpu::flat::GpuFlatField>> = Vec::new();
        let mut original = None;
        for gain in [
            glam::DVec3::ONE,
            glam::DVec3::new(0.8, 1.1, 1.2),
            glam::DVec3::new(1.2, 0.9, 1.1),
        ] {
            let field = if gain == glam::DVec3::ONE {
                project.set_flat_field(None).unwrap();
                None
            } else {
                let map = Arc::new(
                    crate::flat_field::FlatField::from_nodes(
                        source.development.active_area,
                        [2, 2],
                        vec![gain; 4],
                    )
                    .unwrap(),
                );
                project
                    .set_flat_field(Some(crate::project::FlatFieldSettings::new(
                        source.path.clone(),
                        map.clone(),
                    )))
                    .unwrap();
                let gpu = Arc::new(crate::gpu::flat::GpuFlatField::new(&device, map).unwrap());
                fields.push(Arc::downgrade(&gpu));
                Some(gpu)
            };
            let recipe = project.image_recipe(id).unwrap();
            let photograph = image.recipe_calculation(&recipe, field.clone()).unwrap();
            let inspection = image
                .inspection(ProcessingParameters::source(), field)
                .unwrap();
            let codes = camera_cells(photograph.clone());
            assert!(Arc::ptr_eq(&codes, &camera_cells(inspection.clone())));
            if let Some(original) = &original {
                assert!(
                    Arc::ptr_eq(original, &codes),
                    "correction must not replace uploaded camera samples or their coordinate declaration"
                );
            } else {
                original = Some(codes);
            }
            let actual = pixels(&renderer, &queue, inspection.clone());
            assert_eq!(actual.len(), 35);
            let expected = glam::DVec3::new(1100., 900., 750.) / 65535. * gain;
            for pixel in actual {
                for channel in 0..3 {
                    assert!(
                        (f64::from(pixel[channel]) - expected[channel]).abs() < 2e-6,
                        "gain={gain:?} actual={pixel:?} expected={expected:?}"
                    );
                }
            }
            photographs.push(photograph);
            inspections.push(inspection);
        }
        assert!(!photographs[0].same_definition(&photographs[1]));
        assert!(!photographs[1].same_definition(&photographs[2]));
        assert!(
            fields.iter().all(|field| field.upgrade().is_some()),
            "calculations own correction resources after their authoring arguments retire"
        );
        let first_pixels = pixels(&renderer, &queue, inspections[0].clone());
        let last_pixels = pixels(&renderer, &queue, inspections[2].clone());
        assert_ne!(
            first_pixels, last_pixels,
            "the correction control must change pixels"
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        for photograph in &photographs {
            assert!(
                image
                    .prepare_calculation(&device, &mut encoder, photograph, &Default::default())
                    .unwrap()
            );
            assert!(
                !image
                    .prepare_calculation(&device, &mut encoder, photograph, &Default::default())
                    .unwrap(),
                "the unchanged calculation still reuses its prepared result"
            );
        }
        image
            .prepare_calculation(&device, &mut encoder, &inspections[0], &Default::default())
            .unwrap();
        drop((photographs, inspections));
        assert!(
            fields.iter().all(|field| field.upgrade().is_none()),
            "the camera image does not keep project correction resources alive"
        );
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    });
}
