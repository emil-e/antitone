use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn domain(meaning: Meaning) -> Domain {
    Domain {
        bounds: Some([4, 5, 8, 6].into()),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning,
    }
}
#[derive(Clone)]
struct Codes {
    capture: u32,
    reads: Arc<AtomicUsize>,
    values: Arc<Vec<[u16; 4]>>,
}
impl Codes {
    fn new(capture: u32) -> Self {
        Self {
            capture,
            reads: Arc::new(AtomicUsize::new(0)),
            values: Arc::new(
                (0..6)
                    .flat_map(|y| {
                        (0..8).map(move |x| {
                            [
                                if x == 0 { u16::MAX } else { x * 5000 + 3 },
                                y * 9000 + 655 + capture as u16,
                                (x + y) * 4000,
                                x * 55 + y * 17,
                            ]
                        })
                    })
                    .collect(),
            ),
        }
    }
}
impl Source for Codes {
    fn domain(&self) -> Domain {
        domain(Meaning::CameraCodes)
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new((self.capture, self.values.clone())))
    }
    fn storage_format(&self) -> ImageFormat {
        ImageFormat::Rgba16Uint
    }
    fn read_codes(&self, region: Rect) -> Result<Vec<[u16; 4]>, Error> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        let mut values = Vec::new();
        for y in region.y..region.bottom() {
            let start = ((y - 5) * 8 + region.x - 4) as usize;
            values.extend_from_slice(&self.values[start..start + region.width as usize]);
        }
        Ok(values)
    }
    fn read(&self, region: Rect) -> Result<Vec<[f32; 4]>, Error> {
        Ok(self
            .read_codes(region)?
            .into_iter()
            .map(|value| value.map(f32::from))
            .collect())
    }
}
struct Float(Arc<Vec<[f32; 4]>>);
impl Source for Float {
    fn domain(&self) -> Domain {
        domain(Meaning::Neutral)
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(self.0.clone()))
    }
    fn read(&self, region: Rect) -> Result<Vec<[f32; 4]>, Error> {
        let mut values = Vec::new();
        for y in region.y..region.bottom() {
            let start = ((y - 5) * 8 + region.x - 4) as usize;
            values.extend_from_slice(&self.0[start..start + region.width as usize]);
        }
        Ok(values)
    }
}
fn request(context: &Context, image: Arc<ImageNode>, region: Rect) -> Evaluation {
    context
        .request(
            image.clone(),
            Region {
                bounds: region,
                grid: image.domain.grid,
            },
            Budget {
                tile: [region.width, region.height],
                scratch_bytes: 4 << 20,
            },
        )
        .unwrap()
}
#[test]
fn a_live_containing_region_supplies_an_unbounded_calculation_without_source_io() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.test_materializations(false);
    let codes = Codes::new(7);
    let root = reconstruct(
        source(Arc::new(codes.clone())),
        Domain {
            bounds: None,
            ..domain(Meaning::CameraCodes)
        },
    )
    .unwrap();
    let large = pollster::block_on(
        context
            .materialize(
                &queue,
                &Default::default(),
                &scope,
                request(&context, root.clone(), [3, 4, 10, 8].into()),
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap();
    let before = codes.reads.load(Ordering::Relaxed);
    assert!(
        before > 0,
        "cold control must actually acquire source samples"
    );
    let small = pollster::block_on(
        context
            .materialize(
                &queue,
                &Default::default(),
                &scope,
                request(&context, root, [4, 5, 8, 6].into()),
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        codes.reads.load(Ordering::Relaxed),
        before,
        "the held containing value remains usable without optional retention"
    );
    assert_eq!(context.retained_bytes(), 0);
    let stored = execute::read(&device, &queue, small.as_buffer().unwrap());
    let region = small.descriptor.region;
    let mut actual = Vec::new();
    for y in 5..11 {
        let first = (y - region.y) as usize * region.width as usize + (4 - region.x) as usize;
        actual.extend_from_slice(&stored[first..first + 8]);
    }
    let expected: Vec<_> = codes
        .values
        .iter()
        .map(|pixel| pixel.map(f32::from))
        .collect();
    assert_eq!(actual, expected);
    drop(large);
}

fn roundtrip(
    context: &Context,
    queue: &wgpu::Queue,
    scope: &Materializations,
    bound: Arc<ImageNode>,
) -> Image<ImageLayout> {
    let original = bound.domain;
    let output = Domain {
        bounds: Some(crate::gpu::evaluation::CellRegion::new(
            [0, 0],
            [
                original.finite().unwrap().width,
                original.finite().unwrap().height,
            ],
        )),
        grid: Grid::axis_aligned(
            [
                f64::from(original.finite().unwrap().x),
                f64::from(original.finite().unwrap().y),
            ],
            [1.; 2],
        ),
        meaning: original.meaning,
    };
    let root = super::super::reconstruction::reconstruct(bound, output).unwrap();
    let evaluation = context
        .request(
            root,
            Region {
                bounds: output.finite().unwrap(),
                grid: output.grid,
            },
            Budget {
                tile: [
                    output.finite().unwrap().width,
                    output.finite().unwrap().height,
                ],
                scratch_bytes: 16 << 20,
            },
        )
        .unwrap();
    pollster::block_on(
        context
            .materialize(
                queue,
                &Default::default(),
                scope,
                evaluation,
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn native_camera_textures_preserve_each_code_and_cannot_bind_to_another_capture() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let data = Codes::new(7);
    let original = source(Arc::new(data.clone()));
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.test_materializations(false);
    let input = pollster::block_on(
        context
            .materialize(
                &queue,
                &Default::default(),
                &scope,
                request(
                    &context,
                    original.clone(),
                    original.domain.finite().unwrap(),
                ),
                ImageFormat::Rgba16Uint,
                ImageBacking::Texture,
            )
            .unwrap(),
    )
    .unwrap();
    assert!(input.as_texture().is_some());
    let before = data.reads.load(Ordering::Relaxed);
    let result = roundtrip(&context, &queue, &scope, original.clone());
    let actual = execute::read(&device, &queue, result.as_buffer().unwrap());
    assert_eq!(
        actual,
        data.values
            .iter()
            .map(|pixel| pixel.map(f32::from))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        data.reads.load(Ordering::Relaxed),
        before,
        "ready native storage supplies the unchanged graph"
    );
    let other = Codes::new(8);
    let result = roundtrip(&context, &queue, &scope, source(Arc::new(other.clone())));
    assert_eq!(
        execute::read(&device, &queue, result.as_buffer().unwrap()),
        other
            .values
            .iter()
            .map(|pixel| pixel.map(f32::from))
            .collect::<Vec<_>>()
    );
    assert!(
        other.reads.load(Ordering::Relaxed) > 0,
        "another capture must acquire its own values"
    );
    drop(input);
    assert_eq!(context.retained_bytes(), 0);
}

#[test]
fn split_continuous_textures_preserve_fractional_signed_and_above_code_range_values() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let values = Arc::new(
        (0..48)
            .map(|i| {
                [
                    -0.125 - i as f32 * 0.25,
                    65536.5 + i as f32 * 0.125,
                    2_000_000.0 + 0.25 + i as f32 * 0.25,
                    8.,
                ]
            })
            .collect::<Vec<_>>(),
    );
    let original = source(Arc::new(Float(values.clone())));
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.test_materializations(false);
    assert!(
        context
            .materialize(
                &queue,
                &Default::default(),
                &scope,
                request(
                    &context,
                    original.clone(),
                    original.domain.finite().unwrap()
                ),
                ImageFormat::Rgba16Uint,
                ImageBacking::Texture
            )
            .is_err()
    );
    let inputs: Vec<_> = [
        Rect {
            width: 4,
            ..original.domain.finite().unwrap()
        },
        Rect {
            x: 8,
            width: 4,
            ..original.domain.finite().unwrap()
        },
    ]
    .into_iter()
    .map(|region| {
        pollster::block_on(
            context
                .materialize(
                    &queue,
                    &Default::default(),
                    &scope,
                    request(&context, original.clone(), region),
                    ImageFormat::Rgba32Float,
                    ImageBacking::Texture,
                )
                .unwrap(),
        )
        .unwrap()
    })
    .collect();
    let result = roundtrip(&context, &queue, &scope, original);
    assert_eq!(
        execute::read(&device, &queue, result.as_buffer().unwrap()),
        *values,
        "storage transport neither clips signed/HDR values nor quantizes continuous evidence"
    );
    assert_eq!(context.retained_bytes(), 0);
    drop(inputs);
}

#[test]
fn source_texture_uploads_are_bounded_and_preserve_every_row_across_strips() {
    struct Rows {
        domain: Domain,
        reads: Arc<std::sync::Mutex<Vec<Rect>>>,
    }
    impl Source for Rows {
        fn domain(&self) -> Domain {
            self.domain
        }
        fn identity(&self) -> Option<RetainedKey> {
            Some(RetainedKey::new((self.domain, "affine row fixture")))
        }
        fn read(&self, region: Rect) -> Result<Vec<[f32; 4]>, Error> {
            self.reads.lock().unwrap().push(region);
            Ok((region.y..region.bottom())
                .flat_map(|y| {
                    (region.x..region.right()).map(move |x| {
                        [
                            x as f32 + 0.125,
                            -(y as f32) - 0.5,
                            100_000.25 + y as f32 + x as f32,
                            8.,
                        ]
                    })
                })
                .collect())
        }
    }
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.test_materializations(false);
    let bounds: Rect = [17, 19, 1024, 300].into();
    let reads = Arc::new(std::sync::Mutex::new(Vec::new()));
    let original = source(Arc::new(Rows {
        domain: Domain {
            bounds: Some(bounds),
            ..domain(Meaning::Neutral)
        },
        reads: reads.clone(),
    }));
    let input = pollster::block_on(
        context
            .materialize(
                &queue,
                &Default::default(),
                &scope,
                request(&context, original.clone(), bounds),
                ImageFormat::Rgba32Float,
                ImageBacking::Texture,
            )
            .unwrap(),
    )
    .unwrap();
    {
        let reads = reads.lock().unwrap();
        assert!(
            reads.len() > 1,
            "the fixture must exceed the upload working-set cap"
        );
        assert!(
            reads
                .iter()
                .all(|region| u64::from(region.width) * u64::from(region.height) * 16 <= 4 << 20)
        );
        assert_eq!(
            reads
                .iter()
                .map(|region| u64::from(region.width) * u64::from(region.height))
                .sum::<u64>(),
            u64::from(bounds.width) * u64::from(bounds.height)
        );
    }
    let image = roundtrip(&context, &queue, &scope, original);
    assert!(input.as_texture().is_some());
    let values = execute::read(&device, &queue, image.as_buffer().unwrap());
    for y in 0..bounds.height {
        for x in [0, 7, bounds.width - 1] {
            let sx = (bounds.x + x as i32) as f32;
            let sy = (bounds.y + y as i32) as f32;
            assert_eq!(
                values[(y * bounds.width + x) as usize],
                [sx + 0.125, -sy - 0.5, 100_000.25 + sy + sx, 8.],
                "uploaded source ({x},{y})"
            );
        }
    }
}

#[test]
fn incomplete_materialization_is_a_miss_not_an_incomplete_image() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let data = Codes::new(7);
    let original = source(Arc::new(data.clone()));
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.test_materializations(false);
    let area = original.domain.finite().unwrap();
    let part = Rect {
        width: area.width / 2,
        ..area
    };
    let input = pollster::block_on(
        context
            .materialize(
                &queue,
                &Default::default(),
                &scope,
                request(&context, original.clone(), part),
                ImageFormat::Rgba16Uint,
                ImageBacking::Texture,
            )
            .unwrap(),
    )
    .unwrap();
    let before = data.reads.load(Ordering::Relaxed);
    let result = roundtrip(&context, &queue, &scope, original);
    assert_eq!(
        execute::read(&device, &queue, result.as_buffer().unwrap()),
        data.values
            .iter()
            .map(|pixel| pixel.map(f32::from))
            .collect::<Vec<_>>()
    );
    assert!(
        data.reads.load(Ordering::Relaxed) > before,
        "uncovered input falls back to actual source acquisition"
    );
    drop(input);
}

#[test]
fn a_larger_stored_extent_supplies_a_smaller_demand_without_source_reads() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let data = Codes::new(7);
    let original = source(Arc::new(data.clone()));
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.test_materializations(false);
    let image = pollster::block_on(
        context
            .materialize(
                &queue,
                &Default::default(),
                &scope,
                request(
                    &context,
                    original.clone(),
                    original.domain.finite().unwrap(),
                ),
                ImageFormat::Rgba16Uint,
                ImageBacking::Texture,
            )
            .unwrap(),
    )
    .unwrap();
    let before = data.reads.load(Ordering::Relaxed);
    let output = Domain {
        bounds: Some([0, 0, 3, 2].into()),
        grid: Grid::axis_aligned([6., 7.], [1.; 2]),
        meaning: Meaning::CameraCodes,
    };
    let root = super::super::reconstruction::reconstruct(original, output).unwrap();
    let result = pollster::block_on(
        context
            .materialize(
                &queue,
                &Default::default(),
                &scope,
                request(&context, root, output.finite().unwrap()),
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap();
    let actual = execute::read(&device, &queue, result.as_buffer().unwrap());
    let expected = (2..4)
        .flat_map(|y| (2..5).map(move |x| y * 8 + x))
        .map(|index| data.values[index].map(f32::from))
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
    assert_eq!(data.reads.load(Ordering::Relaxed), before);
    drop(image);
}

#[test]
fn stored_domain_and_representation_are_validated_before_source_acquisition() {
    let (device, _queue, _) = crate::gpu::test_support::render_context();
    let data = Codes::new(7);
    let original = source(Arc::new(data.clone()));
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    for bad_domain in [true, false] {
        let scope = context.test_materializations(false);
        let mut layout = ImageLayout {
            domain: original.domain,
            region: original.domain.finite().unwrap(),
            format: ImageFormat::Rgba32Float,
            backing: ImageBacking::Buffer,
        };
        if bad_domain {
            layout.domain.meaning = Meaning::Graded;
        } else {
            layout.backing = ImageBacking::Texture;
        }
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: layout.bytes(),
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let image = pollster::block_on(scope.obtain(
            original.identity().unwrap().clone(),
            Arc::new(layout),
            layout.bytes(),
            move || async move { Ok(GpuImage::new(buffer)) },
        ))
        .unwrap();
        assert!(
            context
                .record(
                    request(
                        &context,
                        original.clone(),
                        original.domain.finite().unwrap()
                    ),
                    scope,
                    Vec::new()
                )
                .is_err()
        );
        assert_eq!(data.reads.load(Ordering::Relaxed), 0);
        drop(image);
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Gain(f32);

impl Operation for Gain {
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(self.clone()))
    }
    fn input(&self, region: Rect) -> Result<Option<Rect>, Error> {
        Ok(Some(region))
    }
    fn pointwise(&self) -> Option<Function> {
        Some(
            Function::new(
                r#"
@group(0) @binding(0) var<uniform> gain: vec4f;
fn scale(pixel: vec4f, position: vec2f) -> vec4f {
    return vec4f(pixel.rgb * gain.x, pixel.w);
}
"#,
                "scale",
            )
            .with_uniform(0, &[self.0, 0., 0., 0.]),
        )
    }
}

#[test]
fn a_cached_intermediate_stops_fusion_and_recording_owns_its_shared_image() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let data = Codes::new(7);
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.test_materializations(false);
    let original = source(Arc::new(data.clone()));
    let first = ImageNode::new(
        Domain {
            meaning: Meaning::Neutral,
            ..original.domain
        },
        Some(original),
        Gain(2.),
    );
    let image = pollster::block_on(
        context
            .materialize(
                &queue,
                &Default::default(),
                &scope,
                request(&context, first.clone(), first.domain.finite().unwrap()),
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap();
    let weak = Arc::downgrade(&image.image);
    let reads = data.reads.load(Ordering::Relaxed);
    let second = ImageNode::new(first.domain, Some(first.clone()), Gain(-0.5));
    let recorded = context
        .record(
            request(&context, second, first.domain.finite().unwrap()),
            scope.clone(),
            Vec::new(),
        )
        .unwrap();
    drop(image);
    assert!(
        weak.upgrade().is_some(),
        "recorded bindings retain the actual shared image owner"
    );
    assert!(
        scope
            .find_ready::<ImageLayout>(first.identity().unwrap(), |_| true)
            .is_some(),
        "active recorded storage stays discoverable with zero optional retention"
    );
    let mut encoder = device.create_command_encoder(&Default::default());
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cached-intermediate consumer output"),
        size: first.domain.finite().unwrap().width as u64
            * first.domain.finite().unwrap().height as u64
            * 16,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    recorded
        .encode_result(&device, &mut encoder, &output)
        .unwrap();
    queue.submit([encoder.finish()]);
    let actual = execute::read(&device, &queue, &output);
    for (input, output) in data.values.iter().zip(actual) {
        assert_eq!(
            output,
            [
                -f32::from(input[0]),
                -f32::from(input[1]),
                -f32::from(input[2]),
                f32::from(input[3])
            ]
        );
    }
    assert_eq!(
        data.reads.load(Ordering::Relaxed),
        reads,
        "fusion must not traverse through the supplied intermediate"
    );
    assert_eq!(context.retained_bytes(), 0);
    drop(recorded);
    assert!(
        weak.upgrade().is_none(),
        "completed required ownership does not become optional history"
    );
}
