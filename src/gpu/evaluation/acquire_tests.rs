use super::*;
use wgpu::util::DeviceExt;

mod axis;

fn buffer(device: &wgpu::Device, values: &[[f32; 4]]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("acquisition test pixels"),
        contents: bytemuck::cast_slice(values),
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
    })
}

fn texture(device: &wgpu::Device, size: [u32; 2]) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("acquisition test texture"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

#[test]
fn partial_presentation_maps_coverage_clears_old_padding_and_preserves_supplied_pixels() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let context = renderer.evaluation();
    let full = buffer(&device, &[[9., 9., 9., 8.]; 12]);
    let values = [
        [-0.5, 2., 0.25, 9.],
        [0.5, -1., 3., 12.],
        [1., 0., -2., 0.],
        [2., 0.5, 1., -1.],
    ];
    let expected = [
        [-0.5, 2., 0.25, 1.],
        [0.5, -1., 3., 1.],
        [0.; 4],
        [2., 0.5, 1., -1.],
    ];
    let source = buffer(&device, &values);
    let texture = texture(&device, [4, 3]);
    let output = buffer(&device, &[[0.; 4]; 48]);
    let mut encoder = device.create_command_encoder(&Default::default());
    acquire::present_sized(context, &mut encoder, &full, [4, 3], &texture).unwrap();
    acquire::present_sized(context, &mut encoder, &source, [2, 2], &texture).unwrap();
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &output,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(3),
            },
        },
        wgpu::Extent3d {
            width: 4,
            height: 3,
            depth_or_array_layers: 1,
        },
    );
    context
        .submit(&queue, &Default::default(), encoder, || false)
        .unwrap()
        .unwrap();
    let read = execute::read(&device, &queue, &output);
    for y in 0..3 {
        for x in 0..4 {
            assert_eq!(
                read[y * 16 + x],
                if y < 2 && x < 2 {
                    expected[y * 2 + x]
                } else {
                    [0.; 4]
                }
            );
        }
    }
    assert_eq!(execute::read(&device, &queue, &source), values);
}

#[test]
fn inspection_transforms_only_covered_signed_hdr_light_and_preserves_floor_flags() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let raster = crate::gpu::raster::Raster::frame(
        crate::frame_raster::FrameRaster::full(
            crate::geometry::Composition::new(
                [11, 17, 3, 2].into(),
                Default::default(),
                Default::default(),
            )
            .unwrap(),
        )
        .unwrap(),
    );
    let domain = Domain {
        bounds: Some([0, 0, 3, 2].into()),
        grid: Grid::from_raster(raster),
        meaning: Meaning::Neutral,
    };
    let values = [
        [-0.5, 2., 0.25, 9.],
        [4., -2., 3., 0.],
        [0., 0., 0., 8.],
        [2., 0.5, -1., 15.],
        [1., 3., 2., 0.],
        [-1., 0.25, 2., 8.],
    ];
    let source = buffer(&device, &values);
    let output = buffer(&device, &[[0.; 4]; 6]);
    let matrix = glam::DMat3::from_cols(
        glam::DVec3::new(1.5, 0.25, -0.125),
        glam::DVec3::new(0.125, 0.5, 0.25),
        glam::DVec3::new(-0.25, 0.125, 2.),
    );
    let root = inspect(
        borrowed_input(domain),
        crate::gpu::parameters::ProcessingParameters::source_with_matrix(matrix).unwrap(),
    )
    .unwrap();
    let prepared = crate::gpu::pixel_pipeline::Prepared::from_composition(
        &device,
        raster,
        root,
        renderer.evaluation.clone(),
    )
    .unwrap();
    let mut encoder = device.create_command_encoder(&Default::default());
    prepared
        .encode_from(&device, &mut encoder, &source, &output)
        .unwrap();
    renderer
        .evaluation
        .submit(&queue, &Default::default(), encoder, || false)
        .unwrap()
        .unwrap();
    for (actual, input) in execute::read(&device, &queue, &output).iter().zip(values) {
        if input[3] as u32 & 8 == 0 {
            assert_eq!(*actual, [0.; 4]);
            continue;
        }
        let expected = matrix * glam::DVec3::new(input[0] as f64, input[1] as f64, input[2] as f64);
        assert_eq!(actual[3], input[3]);
        for c in 0..3 {
            assert!((actual[c] as f64 - expected[c]).abs() < 2e-5);
        }
    }
    assert_eq!(execute::read(&device, &queue, &source), values);
}
