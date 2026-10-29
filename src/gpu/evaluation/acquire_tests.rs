use super::*;
use wgpu::util::DeviceExt;

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
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}
#[test]
fn a_partial_copy_clears_old_padding_and_filter_windows_preserve_unwritten_cells() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let context = renderer.evaluation();
    let full = buffer(&device, &[[9., 9., 9., 7.]; 12]);
    let values = [
        [-0.5, 2., 0.25, 1.],
        [0.5, -1., 3., 2.],
        [1., 0., -2., 4.],
        [2., 0.5, 1., 0.],
    ];
    let source = buffer(&device, &values);
    let texture = texture(&device, [4, 3]);
    let output = buffer(&device, &[[0.; 4]; 12]);
    let mut encoder = device.create_command_encoder(&Default::default());
    acquire::copy(context, &mut encoder, &full, &texture).unwrap();
    acquire::copy_sized(context, &mut encoder, &source, [2, 2], &texture).unwrap();
    acquire::filter(
        context,
        &mut encoder,
        &texture,
        &output,
        acquire::Footprint {
            source: [4, 3],
            output: [4, 3],
            affine: [[0., 0.], [1., 0.], [0., 1.]],
            origin: [0, 0],
            window: ([0, 0], [4, 3]),
        },
    )
    .unwrap();
    context
        .submit(&queue, &Default::default(), encoder, || false)
        .unwrap()
        .unwrap();
    let mut expected = vec![[0., 0., 0., 8.]; 12];
    for y in 0..2 {
        for x in 0..2 {
            let mut p = values[y * 2 + x];
            p[3] = (p[3] as u32 | 8) as f32;
            expected[y * 4 + x] = p;
        }
    }
    assert_eq!(execute::read(&device, &queue, &output), expected);
    assert_eq!(execute::read(&device, &queue, &source), values);

    let sentinel = [0.25, 0.5, -0.25, 1.];
    let output = buffer(&device, &[sentinel; 12]);
    let mut encoder = device.create_command_encoder(&Default::default());
    let window = || acquire::Footprint {
        source: [4, 3],
        output: [4, 3],
        affine: [[0., 0.], [1., 0.], [0., 1.]],
        origin: [0, 0],
        window: ([1, 1], [2, 1]),
    };
    acquire::filter(context, &mut encoder, &texture, &output, window()).unwrap();
    acquire::filter(context, &mut encoder, &texture, &output, window()).unwrap();
    context
        .submit(&queue, &Default::default(), encoder, || false)
        .unwrap()
        .unwrap();
    let actual = execute::read(&device, &queue, &output);
    for (i, pixel) in actual.iter().enumerate() {
        let mut value = sentinel;
        if i == 5 || i == 6 {
            for c in 0..3 {
                value[c] += 2. * expected[i][c];
            }
            value[3] = (value[3] as u32 | expected[i][3] as u32) as f32;
        }
        assert_eq!(*pixel, value);
    }
    let mut encoder = device.create_command_encoder(&Default::default());
    let mut bad = window();
    bad.window = ([3, 2], [2, 1]);
    assert!(acquire::filter(context, &mut encoder, &texture, &output, bad).is_err());
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
        bounds: [0, 0, 3, 2].into(),
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
    let root = Recipe::inspection(
        &device,
        borrowed_input(domain),
        crate::gpu::ProcessingParameters::source_with_matrix(matrix).unwrap(),
    )
    .unwrap();
    let prepared = crate::gpu::pixel_pipeline::Prepared::from_composition(
        &device,
        raster,
        root,
        None,
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
