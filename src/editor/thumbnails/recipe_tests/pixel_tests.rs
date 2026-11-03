use super::*;
use crate::{
    gpu::SubmissionGate,
    looks::{LookAsset, LookOrigin, Lut3D},
    processing::Pipeline,
    rendering::CpuRenderer,
};
use glam::{DMat3, DVec3, Vec3};

// Use the original-sample GPU/OCIO allowance for linear-target comparisons.
const LINEAR_TARGET_TOLERANCE: f32 = 0.0015;

fn linear_pixels(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    submissions: &SubmissionGate,
    image: &GpuImage,
) -> Vec<[f32; 4]> {
    let width = image.linear.width();
    let height = image.linear.height();
    let stride = (width * 16).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("completed thumbnail pixels"),
        size: u64::from(stride) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        image.linear.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(height),
            },
        },
        image.linear.size(),
    );
    let submission = submissions.submit(queue, [encoder.finish()]);
    let (send, receive) = mpsc::sync_channel(1);
    staging.map_async(wgpu::MapMode::Read, .., move |result| {
        send.send(result).unwrap();
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(Duration::from_secs(30)),
        })
        .unwrap();
    receive
        .recv_timeout(Duration::from_secs(30))
        .unwrap()
        .unwrap();
    let mapped = staging.get_mapped_range(..).unwrap();
    let pixels = mapped
        .chunks(stride as usize)
        .flat_map(|row| {
            bytemuck::cast_slice::<u8, [f32; 4]>(&row[..width as usize * 16])
                .iter()
                .copied()
        })
        .collect();
    drop(mapped);
    staging.unmap();
    pixels
}

fn assert_rendered(actual: &[[f32; 4]], expected: &[Vec3]) {
    assert_eq!(actual.len(), expected.len());
    for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        assert_eq!(actual[3], 1., "thumbnail coverage at {index}");
        for channel in 0..3 {
            assert!(
                actual[channel].is_finite()
                    && (actual[channel] - expected[channel]).abs() <= LINEAR_TARGET_TOLERANCE,
                "thumbnail pixel {index}/{channel}: {actual:?} vs {expected:?}"
            );
        }
    }
}

#[test]
fn navigation_reuses_retained_camera_arguments_for_native_and_downstream_edits() {
    let (device, queue) = gpu();
    let renderers = Arc::new(crate::renderer::RendererCache::default());
    let native_reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = native_reads.clone();
    let loader = source_loading::SourceLoader::new_observed(move |_, _, _| {
        observed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Err("native source acquisition is forbidden by this control".into())
    });
    let mut thumbnails = Thumbnails::with_developments(
        device.clone(),
        queue.clone(),
        renderers.clone(),
        None,
        loader.clone(),
    );
    let mut document: crate::editor::document_scope::DocumentScope = tests::fixture().into();
    document.project.set_dmin(Some([0.8; 3]));
    document.project.set_dmax(Some([0.02; 3]));
    let id = document.project.order[0];
    document
        .exposure_slot(&document.exposures, id)
        .set(crate::exposure_reference::ReferenceExposure::Unavailable(
            crate::exposure_reference::UnavailableReference::NoUsablePreview,
        ))
        .unwrap();
    let exposures = document.exposures.clone();
    document.state.capture_exposure(&exposures);
    document
        .project
        .source_mut(id)
        .unwrap()
        .adjustment_layers
        .as_mut()
        .unwrap()
        .layers[0]
        .adjustment
        .photo = AdjustmentLayerSettings {
        exposure: 0.4,
        contrast: 1.15,
        saturation: 1.1,
        ..Default::default()
    };
    let mut layers = document
        .project
        .source(id)
        .unwrap()
        .adjustment_layers
        .clone()
        .unwrap();
    layers
        .add(
            "Gradient".into(),
            crate::adjustment_layers::AdjustmentLayer {
                mask: crate::adjustment_layers::AdjustmentLayerCoverage::Linear {
                    start: crate::geometry::SourcePoint([0., 0.]),
                    end: crate::geometry::SourcePoint([900., 600.]),
                },
                photo: AdjustmentLayerSettings {
                    exposure: -0.7,
                    ..Default::default()
                },
                enabled: true,
                invert: false,
                strength: 1.,
            },
        )
        .unwrap();
    assert_eq!(layers.layers.len(), 2);
    assert_eq!(layers.layers[0].adjustment.photo.exposure, 0.4);
    assert_eq!(layers.layers[1].adjustment.photo.exposure, -0.7);
    document.project.source_mut(id).unwrap().adjustment_layers = Some(layers);
    let size = [8, 6];
    let pixels = (0..size[1])
        .flat_map(|y| {
            (0..size[0]).map(move |x| {
                [
                    if x == 0 && y == 0 {
                        0.
                    } else {
                        5000.25 + x as f32 * 1000.
                    },
                    7000.5 + y as f32 * 1300.,
                    9000.75 + (x + y) as f32 * 350.,
                    0.,
                ]
            })
        })
        .collect::<Vec<_>>();
    thumbnails = thumbnails.with_sources(document.cache.clone());
    thumbnails.seed(
        document.project.source(id).unwrap(),
        FileKey::new(&document.source_path(id).unwrap()),
        Arc::new(Proxy {
            identity: None,
            width: size[0],
            height: size[1],
            pixels: pixels.clone().into(),
            matrix: DMat3::IDENTITY,
        }),
    );
    let look = Arc::new(
        LookAsset::new(
            "Proxy look".into(),
            LookOrigin::Imported,
            Arc::new(
                Lut3D::new(
                    2,
                    [-0.5; 3],
                    [1.1; 3],
                    (0..8)
                        .map(|i| {
                            [
                                0.03 + 0.85 * (i % 2) as f32,
                                0.02 + 0.9 * ((i / 2) % 2) as f32,
                                0.04 + 0.8 * (i / 4) as f32,
                            ]
                        })
                        .collect(),
                )
                .unwrap(),
            ),
        )
        .unwrap(),
    );
    let mut changed_gamma = document.project.input;
    let crate::processing::InputSettings::Negative { gamma, .. } = &mut changed_gamma else {
        panic!("negative fixture");
    };
    *gamma = 0.79;
    for input in [
        document.project.input,
        changed_gamma,
        crate::processing::InputSettings::Digital,
    ] {
        document.project.input = input;
        for target in [OutputTarget::DisplayP3, OutputTarget::Srgb] {
            for selected in [None, Some(look.clone())] {
                document.project.output.lut = selected.clone();
                let recipe = document.project.image_recipe(id).unwrap();
                let pipeline = Pipeline::new(
                    recipe.neutral.processing_input().unwrap(),
                    recipe.roll,
                    DMat3::IDENTITY,
                )
                .unwrap();
                let layers = crate::adjustment_layers::AdjustmentLayerStack::new(
                    recipe.grade.layers().to_vec(),
                )
                .unwrap();
                let expected = pixels
                    .iter()
                    .enumerate()
                    .map(|(index, p)| {
                        let x = index as u32 % size[0];
                        let y = index as u32 / size[0];
                        let point = crate::geometry::SourcePoint([
                            (f64::from(x) + 0.5) * 900. / f64::from(size[0]),
                            (f64::from(y) + 0.5) * 600. / f64::from(size[1]),
                        ]);
                        let camera = DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64) / 65535.;
                        let rgb = layers
                            .apply(pipeline.apply(camera).unwrap(), point)
                            .unwrap()
                            .rgb;
                        selected
                            .as_ref()
                            .map_or(rgb, |look| look.lut().apply(rgb).unwrap())
                    })
                    .collect::<Vec<_>>();
                let expected = CpuRenderer::new(target).unwrap().render(&expected).unwrap();
                let mut key = tests::key(&document, id);
                key.target = target;
                let image = complete(&mut thumbnails, &document, key).unwrap();
                thumbnails.release_selected();
                thumbnails.set_limits(cache_limits::CacheLimits {
                    cpu_bytes: 0,
                    ..thumbnails.limits
                });
                assert_eq!(
                    thumbnails.sources.proxy_count(),
                    0,
                    "subsequent calculations must use retained GPU arguments, not a CPU proxy"
                );
                let actual = linear_pixels(&device, &queue, renderers.submissions(), &image);
                assert_rendered(&actual, &expected);
                assert_eq!(native_reads.load(std::sync::atomic::Ordering::Relaxed), 0);
            }
        }
    }
    loader.close_and_wait().unwrap();
}
