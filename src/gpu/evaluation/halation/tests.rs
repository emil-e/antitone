use super::*;
use crate::{halation::reference, pixel_pipeline::EffectDefinition as _};
use wgpu::util::DeviceExt;

#[derive(Clone)]
struct Authored {
    domain: Domain,
    pixels: Vec<[f32; 4]>,
}
impl Source for Authored {
    fn domain(&self) -> Domain {
        self.domain
    }
    fn read(&self, r: Rect) -> Result<Vec<[f32; 4]>, Error> {
        let a = self.domain.bounds;
        assert!(a.contains_region(r));
        Ok((r.y..r.y + r.height)
            .flat_map(|y| {
                (r.x..r.x + r.width)
                    .map(move |x| self.pixels[((y - a.y) * a.width + x - a.x) as usize])
            })
            .collect())
    }
}
fn context() -> (wgpu::Device, wgpu::Queue) {
    pollster::block_on(async {
        let adapter = crate::gpu::instance()
            .request_adapter(&Default::default())
            .await
            .unwrap();
        adapter.request_device(&Default::default()).await.unwrap()
    })
}
fn fixture(size: [u32; 2], pitch: [f64; 2], coverage: u32, pixels: Vec<[f32; 4]>) -> Authored {
    let mut grid = Grid::axis_aligned([37.25, -20.5], pitch);
    // Rotation and origin must not be mistaken for scale or raster bounds.
    grid.axes = [[0., pitch[0]], [-pitch[1], 0.]];
    grid.coverage = coverage;
    Authored {
        domain: Domain {
            bounds: [11, 17, size[0], size[1]].into(),
            grid,
            meaning: Meaning::Graded,
        },
        pixels,
    }
}
fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    root: Arc<ImageNode>,
    tile: [u32; 2],
) -> Vec<[f32; 4]> {
    let region = root.domain.bounds;
    let plan = Plan::with_policy(
        root,
        region,
        Budget {
            tile,
            scratch_bytes: 16 << 20,
        },
        Policy::Composed,
    )
    .unwrap();
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("halation test readback"),
        size: u64::from(region.width) * u64::from(region.height) * 16,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    Executor::default()
        .run_sink(
            device,
            queue,
            &Default::default(),
            &plan,
            || false,
            |encoder, image, buffer| {
                for row in 0..image.region.height {
                    let dest = ((image.region.y - region.y + row) * region.width + image.region.x
                        - region.x) as u64;
                    encoder.copy_buffer_to_buffer(
                        buffer,
                        u64::from(row * image.region.width) * 16,
                        &staging,
                        dest * 16,
                        u64::from(image.region.width) * 16,
                    );
                }
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    read(device, &staging)
}
fn read(device: &wgpu::Device, staging: &wgpu::Buffer) -> Vec<[f32; 4]> {
    let (send, receive) = std::sync::mpsc::channel();
    staging.map_async(wgpu::MapMode::Read, .., move |result| {
        send.send(result).unwrap()
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(10)),
        })
        .unwrap();
    receive
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap()
        .unwrap();
    let result = bytemuck::cast_slice(&staging.get_mapped_range(..).unwrap()).to_vec();
    staging.unmap();
    result
}
fn assert_reference(actual: &[[f32; 4]], data: &Authored, parameters: HalationParameters) {
    let sigma = data
        .domain
        .grid
        .pitch
        .map(|p| f64::from(parameters.sigma as f32 / p as f32));
    let expected = reference::apply(
        &data
            .pixels
            .iter()
            .map(|p| p.map(f64::from))
            .collect::<Vec<_>>(),
        [
            data.domain.bounds.width as usize,
            data.domain.bounds.height as usize,
        ],
        sigma,
        parameters.strength,
        data.domain.grid.coverage,
    );
    for (index, (a, b)) in actual.iter().zip(expected).enumerate() {
        for c in 0..3 {
            assert!(
                (f64::from(a[c]) - b[c]).abs() < 3e-6 * b[c].abs().max(1.),
                "pixel {index} channel {c}: {} != {}",
                a[c],
                b[c]
            );
        }
        assert_eq!(
            a[2], data.pixels[index][2],
            "blue light is not redistributed at {index}"
        );
        assert_eq!(a[3], b[3] as f32, "metadata {index}");
    }
}

#[test]
fn tuned_default_impulse_matches_oracle_across_tile_boundaries() {
    let (device, queue) = context();
    let size = [41, 41];
    let mut pixels = vec![[0.; 4]; 41 * 41];
    pixels[20 * 41 + 20] = [2.88, 2.88, 2.88, 7.];
    let data = fixture(size, [1.; 2], 0, pixels);
    let parameters = crate::halation::HalationSettings {
        enabled: true,
        ..Default::default()
    }
    .parameters([3840, 2160])
    .unwrap()
    .unwrap();
    let root = halation(source(Arc::new(data.clone())), parameters).unwrap();
    let tiled = render(&device, &queue, root.clone(), [7, 5]);
    assert_reference(&tiled, &data, parameters);
    assert_eq!(tiled, render(&device, &queue, root, size));
}

#[test]
fn completed_signed_mtf_precedes_positive_light_extraction() {
    let (device, queue) = context();
    let size = [23, 19];
    let pixels = (0..size[0] * size[1])
        .map(|i| {
            let x = i % size[0];
            let y = i / size[0];
            [
                if x < 11 { -4. } else { 8. },
                if y < 9 { 2. } else { -3. },
                0.18,
                3.,
            ]
        })
        .collect();
    let data = fixture(size, [1.; 2], 0, pixels);
    let frame = [3840, 2160];
    let mtf_sigma = crate::film_emulation::FilmPreset::ThirtyFiveMm
        .settings()
        .parameters(frame)
        .unwrap()
        .unwrap()
        .image_mtf_sigma;
    let parameters = crate::halation::HalationSettings {
        enabled: true,
        ..Default::default()
    }
    .parameters(frame)
    .unwrap()
    .unwrap();
    let radius = (4. * mtf_sigma).ceil() as i32;
    let normalization: f64 = (-radius..=radius)
        .map(|i| (-0.5 * (f64::from(i) / mtf_sigma).powi(2)).exp())
        .sum();
    let blur = |values: &[[f64; 4]]| -> Vec<[f64; 4]> {
        (0..size[0] * size[1])
            .map(|index| {
                let x = (index % size[0]) as i32;
                let y = (index / size[0]) as i32;
                let mut result = [0.; 4];
                for dy in -radius..=radius {
                    for dx in -radius..=radius {
                        let xx = (x + dx).clamp(0, size[0] as i32 - 1) as u32;
                        let yy = (y + dy).clamp(0, size[1] as i32 - 1) as u32;
                        let w = (-0.5 * f64::from(dx * dx + dy * dy) / mtf_sigma.powi(2)).exp()
                            / normalization.powi(2);
                        for c in 0..3 {
                            result[c] += values[(yy * size[0] + xx) as usize][c] * w;
                        }
                    }
                }
                result[3] = values[index as usize][3];
                result
            })
            .collect()
    };
    let original: Vec<_> = data.pixels.iter().map(|p| p.map(f64::from)).collect();
    let mtf = blur(&original);
    let expected = reference::apply(
        &mtf,
        [size[0] as usize, size[1] as usize],
        [parameters.sigma; 2],
        parameters.strength,
        0,
    );
    let wrong = blur(&reference::apply(
        &original,
        [size[0] as usize, size[1] as usize],
        [parameters.sigma; 2],
        parameters.strength,
        0,
    ));
    assert!(
        expected
            .iter()
            .zip(wrong)
            .any(|(a, b)| (a[0] - b[0]).abs() > 0.01),
        "signed fixture distinguishes otherwise commuting Gaussians"
    );
    let input = gaussian(source(Arc::new(data)), mtf_sigma).unwrap();
    let actual = render(
        &device,
        &queue,
        halation(input, parameters).unwrap(),
        [11, 9],
    );
    for (a, b) in actual.iter().zip(expected) {
        for c in 0..3 {
            assert!((f64::from(a[c]) - b[c]).abs() < 3e-6);
        }
    }
}

#[test]
fn signed_hdr_and_declared_pitch_match_independent_reference_across_tiles_and_edges() {
    let (device, queue) = context();
    let size = [43, 37];
    let pixels: Vec<_> = (0..size[0] * size[1])
        .map(|i| {
            let x = i % size[0];
            let y = i / size[0];
            let base = if (x == 21 || x == 0) && y == 18 {
                64.
            } else {
                0.18
            };
            [
                base,
                if x < 19 { -0.3 } else { base * 0.5 },
                if y < 16 { -1. } else { base * 2. },
                (i % 8) as f32,
            ]
        })
        .collect();
    for pitch in [[1.; 2], [2.25, 3.5]] {
        let data = fixture(size, pitch, 0, pixels.clone());
        for (sigma, strength) in [
            (0.08, 0.2),
            (0.8493218002880191, 0.2),
            (2.5, 0.05),
            (7.5, 0.4),
        ] {
            let p = HalationParameters { sigma, strength };
            let root = p.compose(source(Arc::new(data.clone()))).unwrap();
            let whole = render(&device, &queue, root.clone(), size);
            assert_reference(&whole, &data, p);
            let tiled = render(&device, &queue, root, [13, 11]);
            assert_eq!(
                whole, tiled,
                "real neighbours across tiles, sigma {sigma} pitch {pitch:?}"
            );
        }
    }
}

#[test]
fn constants_negative_sources_and_coverage_have_distinct_contracts() {
    let (device, queue) = context();
    let size = [17, 13];
    let parameters = HalationParameters {
        sigma: 2.,
        strength: 0.2,
    };
    for value in [
        [0.; 3],
        [0.18, 64., 2.],
        [-4., -2., -1.],
        [-1., 0., 4.],
        [f32::MAX * 0.5; 3],
    ] {
        let data = fixture(
            size,
            [1.; 2],
            0,
            vec![[value[0], value[1], value[2], 7.]; (size[0] * size[1]) as usize],
        );
        let actual = render(
            &device,
            &queue,
            parameters.compose(source(Arc::new(data.clone()))).unwrap(),
            size,
        );
        assert_eq!(actual, data.pixels);
    }
    let pixels = (0..size[0] * size[1])
        .map(|i| {
            let x = i % size[0];
            let y = i / size[0];
            if x + y < 6 || x + y > 22 {
                [1000., 3000., 2000., 0.]
            } else {
                [0.18, 0.18, 0.18, 8.]
            }
        })
        .collect();
    let data = fixture(size, [1.; 2], 8, pixels);
    let output = render(
        &device,
        &queue,
        parameters.compose(source(Arc::new(data.clone()))).unwrap(),
        size,
    );
    assert_reference(&output, &data, parameters);
    for (i, p) in output.iter().enumerate().filter(|(_, p)| p[3] == 8.) {
        assert_eq!(p, &data.pixels[i], "uncovered cells must not seed a halo");
    }
}

#[test]
fn exposure_homogeneity_and_negative_only_identity_hold_in_production() {
    let (device, queue) = context();
    let size = [21, 19];
    let pixels = (0..399)
        .map(|i| {
            [
                if i % 7 == 0 { 8. } else { -0.2 },
                if i % 11 == 0 { 4. } else { 0.18 },
                -1.,
                3.,
            ]
        })
        .collect();
    let data = fixture(size, [1.; 2], 0, pixels);
    let p = HalationParameters {
        sigma: 1.3,
        strength: 0.2,
    };
    let baseline = render(
        &device,
        &queue,
        p.compose(source(Arc::new(data.clone()))).unwrap(),
        size,
    );
    for gain in [0.125, 2., 64.] {
        let mut scaled = data.clone();
        for pixel in &mut scaled.pixels {
            for v in &mut pixel[..3] {
                *v *= gain;
            }
        }
        let actual = render(
            &device,
            &queue,
            p.compose(source(Arc::new(scaled))).unwrap(),
            size,
        );
        for (a, b) in actual.iter().zip(&baseline) {
            for c in 0..3 {
                assert!((a[c] - b[c] * gain).abs() < 1e-6 * gain);
            }
        }
    }
    let mut negative = data;
    for pixel in &mut negative.pixels {
        for v in &mut pixel[..3] {
            *v = -v.abs();
        }
    }
    let actual = render(
        &device,
        &queue,
        p.compose(source(Arc::new(negative.clone()))).unwrap(),
        size,
    );
    assert_eq!(actual, negative.pixels);
}

#[test]
fn zero_strength_bypasses_and_abandoned_recordings_leave_input_unchanged() {
    let (device, queue) = context();
    let data = fixture([19, 17], [1.; 2], 0, vec![[2., -0.2, 4., 3.]; 323]);
    let input = raster_input(data.domain);
    assert!(Arc::ptr_eq(
        &input,
        &halation(
            input.clone(),
            HalationParameters {
                sigma: 2.,
                strength: 0.
            }
        )
        .unwrap()
    ));
    let root = halation(
        input,
        HalationParameters {
            sigma: 2.,
            strength: 0.2,
        },
    )
    .unwrap();
    let plan = Plan::with_policy(
        root,
        data.domain.bounds,
        Budget {
            tile: [19, 17],
            scratch_bytes: 1 << 20,
        },
        Policy::Composed,
    )
    .unwrap();
    let prepared = Executor::default().prepare(&device, plan, vec![]).unwrap();
    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&data.pixels),
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
    });
    let mut abandoned = device.create_command_encoder(&Default::default());
    prepared.encode(&device, &mut abandoned, &buffer).unwrap();
    drop(abandoned);
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: buffer.size(),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(&buffer, 0, &staging, 0, buffer.size());
    queue.submit([encoder.finish()]);
    assert_eq!(read(&device, &staging), data.pixels);
}
