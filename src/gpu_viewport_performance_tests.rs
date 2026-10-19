use super::*;

fn crop_pixels(
    pixels: &[[f32; 4]],
    size: [u32; 2],
    origin: [u32; 2],
    crop: [u32; 2],
) -> Vec<[f32; 4]> {
    let mut result = Vec::with_capacity((crop[0] * crop[1]) as usize);
    for y in origin[1]..origin[1] + crop[1] {
        let start = (y * size[0] + origin[0]) as usize;
        result.extend_from_slice(&pixels[start..start + crop[0] as usize]);
    }
    result
}

#[allow(clippy::too_many_arguments)]
async fn viewport_probe(
    pixels: Vec<[f32; 4]>,
    size: [u32; 2],
    area: SourceRegion,
    calibration: Calibration,
    matrix: DMat3,
    zoom_pixels: Vec<[f32; 4]>,
    zoom_region: SourceRegion,
    corrected: bool,
) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = instance.request_adapter(&Default::default()).await.unwrap();
    let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
    let kernel = ProcessingKernel::new(&device).await.unwrap();
    let geometry = SourceGeometry::new([area.x, area.y, area.width, area.height], size).unwrap();
    let origin = [size[0] / 4, size[1] / 4];
    let crop_size = [size[0] / 2, size[1] / 2];
    let crop_pixels = crop_pixels(&pixels, size, origin, crop_size);
    let approximate_region = SourceRegion {
        x: area.x + (u64::from(origin[0]) * u64::from(area.width) / u64::from(size[0])) as u32,
        y: area.y + (u64::from(origin[1]) * u64::from(area.height) / u64::from(size[1])) as u32,
        width: (u64::from(crop_size[0]) * u64::from(area.width) / u64::from(size[0])) as u32,
        height: (u64::from(crop_size[1]) * u64::from(area.height) / u64::from(size[1])) as u32,
    };
    let crop_geometry = SourceGeometry::new(
        [
            approximate_region.x,
            approximate_region.y,
            approximate_region.width,
            approximate_region.height,
        ],
        crop_size,
    )
    .unwrap();
    let naive_mismatches = (0..crop_pixels.len())
        .filter(|&i| {
            let original = (origin[1] + i as u32 / crop_size[0]) * size[0]
                + origin[0]
                + i as u32 % crop_size[0];
            crop_geometry.pixel(i as u32).unwrap() != geometry.pixel(original).unwrap()
        })
        .count();
    // Test-only specialization demonstrates original-raster addressing without changing production geometry.
    let old = "let xy = vec2u(index % field.dimensions.x, index / field.dimensions.x);\n    return field.area.xy + ((2u * xy + vec2u(1u)) * field.area.zw) / (2u * field.dimensions.xy);";
    let replacement = format!(
        "let xy = vec2u(index % {}u, index / {}u) + vec2u({}u, {}u);\n    return vec2u({}u, {}u) + ((2u * xy + vec2u(1u)) * vec2u({}u, {}u)) / (2u * vec2u({}u, {}u));",
        crop_size[0],
        crop_size[0],
        origin[0],
        origin[1],
        area.x,
        area.y,
        area.width,
        area.height,
        size[0],
        size[1]
    );
    let shader = ProcessingKernel::shader();
    assert_eq!(shader.matches(old).count(), 1);
    let crop_kernel = shader_kernel(&device, shader.replace(old, &replacement)).await;
    let mut full_source = ProcessingSource::new_continuous(&device, &pixels, matrix).unwrap();
    full_source.set_geometry(geometry).unwrap();
    let mut crop_source = ProcessingSource::new_continuous(&device, &crop_pixels, matrix).unwrap();
    crop_source.set_geometry(crop_geometry).unwrap();
    let zoom_geometry = SourceGeometry::new(
        [
            zoom_region.x,
            zoom_region.y,
            zoom_region.width,
            zoom_region.height,
        ],
        size,
    )
    .unwrap();
    let mut zoom_source = ProcessingSource::new_continuous(&device, &zoom_pixels, matrix).unwrap();
    zoom_source.set_geometry(zoom_geometry).unwrap();
    let calibration = if corrected {
        let field = Arc::new(
            crate::flat_field::FlatField::from_nodes(
                [area.x, area.y, area.width, area.height],
                [2, 2],
                vec![
                    DVec3::new(1.2, 0.9, 1.1),
                    DVec3::new(0.95, 1.3, 1.05),
                    DVec3::new(1.1, 1.05, 0.88),
                    DVec3::new(1.25, 0.98, 1.15),
                ],
            )
            .unwrap(),
        );
        let gpu_field = Arc::new(GpuFlatField::new(&device, field.clone()).unwrap());
        full_source
            .set_flat_field(Some(gpu_field.clone()), geometry)
            .unwrap();
        crop_source
            .set_flat_field(Some(gpu_field.clone()), crop_geometry)
            .unwrap();
        zoom_source
            .set_flat_field(Some(gpu_field), zoom_geometry)
            .unwrap();
        field.processing_calibration(calibration).unwrap()
    } else {
        calibration
    };
    let full_probe = Probe::new(&device, pixels.len()).await;
    let crop_probe = Probe::new(&device, crop_pixels.len()).await;
    // Reuse output storage for same-size full/zoom sources; mutable outputs are not shared across jobs.
    let grade = GpuPhotoGrade::new(
        &device,
        calibration,
        RollSettings::new_grade(),
        snapshot(PhotoSettings::default(), 2, true),
        matrix,
        area,
    )
    .unwrap();
    let run_full = || {
        timed(
            &device,
            &queue,
            &kernel,
            &full_probe,
            &full_source,
            &grade,
            None,
        )
    };
    let run_crop = || {
        timed(
            &device,
            &queue,
            &crop_kernel,
            &crop_probe,
            &crop_source,
            &grade,
            None,
        )
    };
    let run_zoom = || {
        timed(
            &device,
            &queue,
            &kernel,
            &full_probe,
            &zoom_source,
            &grade,
            None,
        )
    };
    run_full();
    let expected = read_words(&device, &queue, &full_probe.output);
    run_crop();
    let actual = read_words(&device, &queue, &crop_probe.output);
    for i in 0..crop_pixels.len() {
        let original =
            (origin[1] + i as u32 / crop_size[0]) * size[0] + origin[0] + i as u32 % crop_size[0];
        assert_eq!(
            &actual[4 * i..4 * i + 4],
            &expected[4 * original as usize..4 * original as usize + 4],
            "ROI pixel={i}"
        );
    }
    run_zoom();
    scalar_samples(
        &device,
        &queue,
        &full_probe,
        &zoom_source,
        &grade,
        calibration,
        &zoom_pixels,
    );
    let mut full = Vec::new();
    let mut crop = Vec::new();
    let mut zoom = Vec::new();
    let mut ratios = Vec::new();
    for round in 0..9 {
        let before = run_full();
        let (c, z) = if round % 2 == 0 {
            (run_crop(), run_zoom())
        } else {
            let z = run_zoom();
            (run_crop(), z)
        };
        let after = run_full();
        crop.push(c);
        zoom.push(z);
        full.extend([before, after]);
        ratios.push(c / ((before + after) / 2.));
    }
    println!(
        "viewport_probe size={size:?} corrected={corrected} retained_proxy_ms={:.3} identical_pitch_crop_size={crop_size:?} crop_ms={:.3} paired_crop_ratio={:.4} equal_budget_zoom_ms={:.3} full_rgb16_bytes={} crop_rgb16_bytes={} zoom_rgb16_bytes={} naive_geometry_mismatches={naive_mismatches} exact_crop_pixels={} scope_full_pass_additional=true excludes=upload_readback_compilation_rendering_reconstruction",
        median(&full),
        median(&crop),
        median(&ratios),
        median(&zoom),
        full_source.byte_len(),
        crop_source.byte_len(),
        zoom_source.byte_len(),
        crop_pixels.len()
    );
}

fn synthetic_pixels(geometry: SourceGeometry, pixels: u32) -> Vec<[f32; 4]> {
    (0..pixels)
        .map(|i| {
            let [x, y] = geometry.pixel(i).unwrap();
            [
                (x * 1871 + y * 29) as u16,
                (x * 7919 + y * 73) as u16,
                (x * 3413 + y * 137) as u16,
                42,
            ]
            .map(f32::from)
        })
        .collect()
}

#[test]
#[ignore = "visible-region source sampling and processing-only costs; no production zoom policy or settled-quality substitution"]
fn visible_region_source_sampling() {
    let a = area();
    let zoom_region = SourceRegion {
        x: a.x + a.width / 4,
        y: a.y + a.height / 4,
        width: a.width / 2,
        height: a.height / 2,
    };
    let zoom_geometry = SourceGeometry::new(
        [
            zoom_region.x,
            zoom_region.y,
            zoom_region.width,
            zoom_region.height,
        ],
        SIZE,
    )
    .unwrap();
    for corrected in [false, true] {
        pollster::block_on(viewport_probe(
            synthetic_pixels(geometry(), SIZE[0] * SIZE[1]),
            SIZE,
            a,
            calibration(),
            camera_matrix(),
            synthetic_pixels(zoom_geometry, SIZE[0] * SIZE[1]),
            zoom_region,
            corrected,
        ));
    }
}

#[cfg(feature = "raw")]
#[test]
#[ignore = "read-only retained-RAW visible-region sampling experiment; not a production live-preview tier"]
fn raw_visible_region_source_sampling() {
    let (scan, calibration) = crate::raw_test_support::train_raw::load();
    let (width, height, pixels) = scan.preview_source(1024).unwrap();
    let a = scan.area;
    let zoom_region = SourceRegion {
        x: a.x + a.width / 4,
        y: a.y + a.height / 4,
        width: a.width / 2,
        height: a.height / 2,
    };
    let raster = crate::geometry::SourceRaster::new(zoom_region, [width, height]).unwrap();
    let start = Instant::now();
    let mut zoom_pixels = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        let [_, sy] = raster.source_pixel([0, y]).unwrap();
        let row = scan.tile_region(a, 0, sy - a.y, a.width, 1).unwrap();
        for x in 0..width {
            let [sx, _] = raster.source_pixel([x, y]).unwrap();
            zoom_pixels.push(row[(sx - a.x) as usize]);
        }
    }
    println!(
        "viewport_raw zoom_camera_sampling_ms={:.3} additional_raw_development=false size=[{width},{height}]",
        start.elapsed().as_secs_f64() * 1000.
    );
    pollster::block_on(viewport_probe(
        pixels,
        [width, height],
        a,
        calibration,
        scan.camera_to_ap1,
        zoom_pixels,
        zoom_region,
        false,
    ));
}
