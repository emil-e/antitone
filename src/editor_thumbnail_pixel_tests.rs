use super::*;
use crate::{
    gpu::SubmissionGate,
    looks::{LookAsset, LookOrigin, Lut3D},
    processing::{InvertedPixel, Pipeline},
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

fn box_average(input: &[DVec3], source: [u32; 2], output: [u32; 2]) -> Vec<DVec3> {
    let [width, height] = source.map(u64::from);
    let [columns, rows] = output.map(u64::from);
    let mut result = Vec::with_capacity((columns * rows) as usize);
    for y in 0..rows {
        let top = y * height;
        let bottom = (y + 1) * height;
        for x in 0..columns {
            let left = x * width;
            let right = (x + 1) * width;
            let mut sum = DVec3::ZERO;
            // Integer edge units give exact box overlaps, independently of GPU affine geometry.
            for sy in top / rows..bottom.div_ceil(rows) {
                let wy = bottom.min((sy + 1) * rows) - top.max(sy * rows);
                for sx in left / columns..right.div_ceil(columns) {
                    let wx = right.min((sx + 1) * columns) - left.max(sx * columns);
                    sum += input[(sy * width + sx) as usize] * (wx * wy) as f64;
                }
            }
            result.push(sum / (width * height) as f64);
        }
    }
    result
}

fn rendered_reference(
    neutral: &[DVec3],
    pipeline: &Pipeline,
    look: Option<&LookAsset>,
    renderer: &CpuRenderer,
) -> Vec<Vec3> {
    let graded = neutral
        .iter()
        .map(|&rgb| {
            let value = pipeline
                .grade_neutral(InvertedPixel {
                    rgb,
                    floored: [false; 3],
                })
                .unwrap()
                .rgb;
            look.map_or(value, |asset| asset.lut().apply(value).unwrap())
        })
        .collect::<Vec<_>>();
    renderer.render(&graded).unwrap()
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
fn authored_thumbnail_pixels_apply_repairs_before_filtering_and_restore_after_disable() {
    let (device, queue) = gpu();
    let renderers = Arc::new(crate::renderer::RendererCache::default());
    let mut thumbnails =
        Thumbnails::with_disk(device.clone(), queue.clone(), renderers.clone(), None);
    let mut document = Document::default();
    document
        .import(&[PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/roll-0.dng")])
        .unwrap();
    document.project.dmin = Some([0.8; 3]);
    document.project.dmax = Some([0.02; 3]);
    document.project.input_floor = Some(crate::processing::CAMERA_INPUT_FLOOR);
    let id = document.project.order[0];
    document.project.source_mut(id).unwrap().photo = PhotoSettings {
        exposure: 0.5,
        contrast: 1.2,
        ..Default::default()
    };
    let plain = complete(&mut thumbnails, &document, tests::key(&document, id)).unwrap();
    let baseline = linear_pixels(&device, &queue, renderers.submissions(), &plain);
    thumbnails.set_limits(cache_limits::CacheLimits {
        cpu_bytes: 0,
        gpu_bytes: 0,
    });
    let output = [plain.linear.width(), plain.linear.height()];
    let recipe = document.project.image_recipe(id).unwrap();
    let scan = Scan::open(
        &document.source_path(id).unwrap(),
        Some(recipe.neutral.development.fixed_wb),
    )
    .unwrap();
    let area = scan.area;
    assert!(output[0] < area.width && output[1] < area.height);

    // The mask crosses the authored film-base/ramp join; a base-side donor changes its hidden curvature.
    // Its left edge splits output footprints, mixing repaired and known original samples.
    let origin = [84, 112];
    assert_ne!((origin[0] - area.x) * output[0] % area.width, 0);
    let repair = document
        .project
        .source_mut(id)
        .unwrap()
        .repairs
        .insert(
            SupportMask::new(origin, [49, 49], vec![true; 49 * 49]).unwrap(),
            RepairOrigin::Manual,
            DonorChoice::Offset([-64, 0]),
        )
        .unwrap();
    let recipe = document.project.image_recipe(id).unwrap();
    let evaluated = crate::repair_source::resolve(&scan, &recipe)
        .unwrap()
        .remove(0);
    let calibration = recipe.neutral.processing_calibration().unwrap();
    let matrix = DMat3::from_cols_array_2d(&recipe.neutral.source_development.camera_to_ap1);
    let neutral = Pipeline::ungraded(calibration, recipe.roll, matrix).unwrap();
    let pipeline = Pipeline::new(calibration, recipe.roll, recipe.grade.global(), matrix).unwrap();
    let originals = (area.y..area.y + area.height)
        .flat_map(|y| (area.x..area.x + area.width).map(move |x| [x, y]))
        .map(|[x, y]| neutral.apply(scan.pixel(x, y).unwrap()).unwrap().rgb)
        .collect::<Vec<_>>();
    let mut replacements = originals.clone();
    for (point, value) in evaluated.pixels() {
        let index = ((point[1] - area.y) * area.width + point[0] - area.x) as usize;
        replacements[index] = DVec3::from_array(value.map(f64::from));
    }
    let original_filtered = box_average(&originals, [area.width, area.height], output);
    let repaired_filtered = box_average(&replacements, [area.width, area.height], output);
    let renderer = CpuRenderer::new(tests::key(&document, id).target).unwrap();
    let nodes = (0..8)
        .map(|i| {
            [
                0.03 + 0.85 * (i % 2) as f32,
                0.02 + 0.9 * ((i / 2) % 2) as f32,
                0.04 + 0.8 * (i / 4) as f32,
            ]
        })
        .collect();
    let look = Arc::new(
        LookAsset::new(
            "Thumbnail scene look".into(),
            LookOrigin::Imported,
            Arc::new(Lut3D::new(2, [0.; 3], [1.; 3], nodes).unwrap()),
        )
        .unwrap(),
    );

    for selected in [None, Some(look)] {
        document.project.output.look = selected.clone();
        document
            .project
            .source_mut(id)
            .unwrap()
            .repairs
            .set_enabled(repair, true)
            .unwrap();
        let repaired = complete(&mut thumbnails, &document, tests::key(&document, id)).unwrap();
        let actual = linear_pixels(&device, &queue, renderers.submissions(), &repaired);
        let expected = rendered_reference(
            &repaired_filtered,
            &pipeline,
            selected.as_deref(),
            &renderer,
        );
        let unrepaired = rendered_reference(
            &original_filtered,
            &pipeline,
            selected.as_deref(),
            &renderer,
        );
        assert_rendered(&actual, &expected);
        assert!(
            expected
                .iter()
                .zip(&unrepaired)
                .any(|(a, b)| (*a - *b).abs().max_element() > 2. * LINEAR_TARGET_TOLERANCE),
            "repair must change rendered pixels beyond twice the GPU/OCIO comparison allowance"
        );

        document
            .project
            .source_mut(id)
            .unwrap()
            .repairs
            .set_enabled(repair, false)
            .unwrap();
        let disabled = complete(&mut thumbnails, &document, tests::key(&document, id)).unwrap();
        let restored = linear_pixels(&device, &queue, renderers.submissions(), &disabled);
        assert_rendered(&restored, &unrepaired);
        if selected.is_none() {
            assert_eq!(
                restored, baseline,
                "disabling the repair restores the original card pixels"
            );
        }
    }
}
