use super::*;
mod assembly;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Codes;
impl Codes {
    fn marked(x: i32, y: i32) -> bool {
        (x - 256).pow(2) + (y - 256).pow(2) <= 12 * 12
    }
    fn value(x: i32, y: i32) -> [u16; 4] {
        if Self::marked(x, y) {
            [0; 4]
        } else {
            [1000, 2000, 4000, 0]
        }
    }
}
impl Source for Codes {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some([0, 0, 512, 512].into()),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::CameraCodes,
        }
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("constant codes with circular defect"))
    }
    fn storage_format(&self) -> ImageFormat {
        ImageFormat::Rgba16Uint
    }
    fn read_codes(&self, region: CellRegion) -> Result<Vec<[u16; 4]>, Error> {
        Ok((region.y..region.bottom())
            .flat_map(|y| (region.x..region.right()).map(move |x| Self::value(x, y)))
            .collect())
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        Ok(self
            .read_codes(region)?
            .into_iter()
            .map(|pixel| pixel.map(f32::from))
            .collect())
    }
}
struct Normalize;
impl Operation for Normalize {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("latency fixture normalization"))
    }
    fn reads(&self, region: CellRegion) -> Result<Vec<Read>, Error> {
        Ok(vec![Read::new(0, region)])
    }
    fn calculation(
        self: std::sync::Arc<Self>,
        inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _budget: crate::gpu::evaluation::Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, crate::Error> {
        let inputs = crate::gpu::evaluation::value::Calculation::references(inputs);
        let function = Function::new(
            "fn normalize(p:vec4f,at:vec2f)->vec4f {return vec4f(p.rgb / 10000.,p.w);}",
            "normalize",
        );
        value::Calculation::new(
            domain,
            output,
            vec![inputs[0].request(output)?],
            value::Operation::shader_function(self.key(), function),
        )
        .map(Some)
    }
}

fn read_frame(
    context: &Context,
    queue: &wgpu::Queue,
    gate: &crate::gpu::submission::SubmissionGate,
    image: Value,
) -> Vec<[f32; 4]> {
    let region = image.domain().finite().unwrap();
    let query = context
        .request(
            image.clone(),
            Region {
                bounds: region,
                grid: image.domain().grid,
            },
            Budget {
                tile: [region.width, region.height],
                scratch_bytes: 512 << 10,
            },
        )
        .unwrap();
    let mut pixels = vec![[0.; 4]; region.width as usize * region.height as usize];
    context
        .read(
            queue,
            gate,
            &query,
            || false,
            |block, values| {
                for row in 0..block.height {
                    let from = row as usize * block.width as usize;
                    let to = region.index([block.x, block.y + row as i32]);
                    pixels[to..to + block.width as usize]
                        .copy_from_slice(&values[from..from + block.width as usize]);
                }
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    pixels
}

#[test]
fn repairing_a_reduced_rotated_view_does_not_materialize_a_source_sized_intermediate() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    for angle in [0., 0.2] {
        let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
        let gate = crate::gpu::submission::SubmissionGate::default();
        let count = Arc::new(AtomicUsize::new(0));
        let observed = count.clone();
        gate.observe_encoded_resources(Arc::new(move || {
            observed.fetch_add(1, Ordering::Relaxed);
        }));
        let camera = source(Arc::new(Codes));
        let query = context
            .request(
                camera.clone(),
                Region {
                    bounds: camera.domain().finite().unwrap(),
                    grid: camera.domain().grid,
                },
                Budget {
                    tile: [512, 128],
                    scratch_bytes: 512 << 10,
                },
            )
            .unwrap();
        // A live camera argument is independent of optional generated-image retention.
        let held = pollster::block_on(
            context
                .materialize(
                    &queue,
                    &gate,
                    &context.materializations(),
                    query,
                    ImageFormat::Rgba16Uint,
                    ImageBacking::Buffer,
                )
                .unwrap(),
        )
        .unwrap();
        let base = Value::from_operation(
            Domain {
                meaning: Meaning::Neutral,
                ..camera.domain()
            },
            [camera],
            Normalize,
        );
        let mask = (240..272)
            .flat_map(|y| (240..272).map(move |x| Codes::marked(x, y)))
            .collect();
        let repair = Repair {
            id: RepairId(1),
            enabled: true,
            origin: RepairOrigin::Manual,
            support: SupportMask::new([240, 240], [32, 32], mask).unwrap(),
            offset: [96, 0],
        };
        let composition = crate::geometry::Composition::with_crop(
            [0, 0, 512, 512].into(),
            Default::default(),
            Default::default(),
            Some(crate::crop::Crop {
                center: [256.; 2],
                size: [384; 2],
                angle,
            }),
        )
        .unwrap();
        let mut frame = crate::frame_raster::FrameRaster::full(composition).unwrap();
        frame.output = [64, 64];
        let domain = Domain {
            bounds: Some([0, 0, 64, 64].into()),
            grid: Grid::from_frame(frame),
            meaning: Meaning::Neutral,
        };
        let damaged = reconstruct(base.clone(), domain).unwrap();
        let control = read_frame(&context, &queue, &gate, damaged);
        assert!(
            control.iter().any(|pixel| pixel[0] < 0.05),
            "the visible defect must distinguish healing from a no-op"
        );
        let healed = reconstruct(compose(base, &[repair]).unwrap(), domain).unwrap();
        count.store(0, Ordering::Relaxed);
        let pixels = read_frame(&context, &queue, &gate, healed);
        for pixel in pixels {
            for (actual, expected) in pixel.into_iter().zip([0.1, 0.2, 0.4, 0.]) {
                assert!(
                    (actual - expected).abs() < 2e-6,
                    "constant donor and boundary restore every output sample, including metadata: {pixel:?}"
                );
            }
        }
        assert!(
            count.load(Ordering::Relaxed) <= 12,
            "one small repair must not turn bounded reconstruction into hundreds of synchronized submissions: {}",
            count.load(Ordering::Relaxed)
        );
        drop(held);
    }
}

struct MarkedPlate;
impl Source for MarkedPlate {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some([0, 0, 256, 128].into()),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::Neutral,
        }
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("six separated dark target cells"))
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        Ok((region.y..region.bottom())
            .flat_map(|y| {
                (region.x..region.right()).map(move |x| {
                    if y == 32 && (0..6).any(|i| x == 16 + i * 32) {
                        [0., 0., 0., 0.75]
                    } else {
                        let stripe = x.div_euclid(32) as f32;
                        [
                            2. + stripe,
                            -0.5 - stripe * 0.125,
                            0.25 + stripe * 0.125,
                            0.75,
                        ]
                    }
                })
            })
            .collect())
    }
}

struct WideBase(u32);
impl Operation for WideBase {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(("wide base arguments", self.0)))
    }
    fn reads(&self, region: CellRegion) -> Result<Vec<Read>, Error> {
        Ok(vec![Read::new(0, region)])
    }
    fn calculation(
        self: std::sync::Arc<Self>,
        inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _budget: crate::gpu::evaluation::Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, crate::Error> {
        let inputs = crate::gpu::evaluation::value::Calculation::references(inputs);
        let mut code = String::new();
        let mut body = String::from("var pixel = p;\n");
        for slot in 0..self.0 {
            code.push_str(&format!(
                "@group(0) @binding({slot}) var<storage,read> extra_{slot}:array<vec4f>;\n"
            ));
            body.push_str(&format!(
                "pixel = vec4f(pixel.rgb + extra_{slot}[0].rgb, pixel.w);\n"
            ));
        }
        code.push_str(&format!(
            "fn wide(p:vec4f,at:vec2f)->vec4f {{ {body} return pixel; }}"
        ));
        let mut function = Function::new(code, "wide");
        for slot in 0..self.0 {
            function =
                function.with_storage(slot, Arc::<[u8]>::from(bytemuck::cast_slice(&[0_f32; 4])));
        }

        let function = function;
        value::Calculation::new(
            domain,
            output,
            vec![inputs[0].request(output)?],
            value::Operation::shader_function(self.key(), function),
        )
        .map(Some)
    }
}

#[test]
fn a_wide_base_retains_the_repair_grade_and_window_binding_fallbacks() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    // Base sampling fits alone; adding packed repair arguments exceeds device capacity.
    let auxiliary = device.limits().max_storage_buffers_per_shader_stage - 2;
    assert!(auxiliary + 4 > device.limits().max_storage_buffers_per_shader_stage);
    for endpoint in ["repair", "grade", "window"] {
        let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
        let raw = source(Arc::new(MarkedPlate));
        let base = Value::from_operation(raw.domain(), [raw], WideBase(auxiliary));
        let repairs = (0..6)
            .map(|index| repair([16 + index * 32, 32], [0, 48]))
            .collect::<Vec<_>>();
        let region: CellRegion = [8, 24, 192, 16].into();
        let collect = |root: Value| {
            let query = context
                .request(
                    root.clone(),
                    Region {
                        bounds: region,
                        grid: root.domain().grid,
                    },
                    Budget {
                        tile: [192, 16],
                        scratch_bytes: 4 << 20,
                    },
                )
                .unwrap();
            let mut pixels = vec![[0.; 4]; 192 * 16];
            context
                .read(
                    &queue,
                    &Default::default(),
                    &query,
                    || false,
                    |block, values| {
                        for row in 0..block.height {
                            let from = row as usize * block.width as usize;
                            let to = region.index([block.x, block.y + row as i32]);
                            pixels[to..to + block.width as usize]
                                .copy_from_slice(&values[from..from + block.width as usize]);
                        }
                        Ok(())
                    },
                )
                .unwrap()
                .unwrap();
            pixels
        };
        let control = collect(base.clone());
        for repair in &repairs {
            let target = repair
                .support
                .selected_points()
                .next()
                .unwrap()
                .map(|p| p as i32);
            assert_eq!(
                control[region.index(target)],
                [0., 0., 0., 0.75],
                "every target must distinguish a dropped replacement"
            );
        }
        let root = compose(base, &repairs).unwrap();
        let root = match endpoint {
            "grade" => pointwise(
                root,
                Meaning::Graded,
                Function::new(
                    "fn twice(p:vec4f,at:vec2f)->vec4f {return vec4f(2.*p.rgb,p.w);}",
                    "twice",
                ),
            ),
            "window" => window(root, region).unwrap(),
            _ => root,
        };
        let colors: [[f32; 4]; 7] = [
            [2., -0.5, 0.25, 0.75],
            [3., -0.625, 0.375, 0.75],
            [4., -0.75, 0.5, 0.75],
            [5., -0.875, 0.625, 0.75],
            [6., -1., 0.75, 0.75],
            [7., -1.125, 0.875, 0.75],
            [8., -1.25, 1., 0.75],
        ];
        let pixels = collect(root);
        for y in region.y..region.bottom() {
            for x in region.x..region.right() {
                let mut expected = colors[x as usize / 32];
                if endpoint == "grade" {
                    for value in &mut expected[..3] {
                        *value *= 2.;
                    }
                }
                assert_eq!(
                    pixels[region.index([x, y])],
                    expected,
                    "{endpoint}: independently colored CPU repairs and untouched stripes must retain original metadata"
                );
            }
        }
    }
}

struct Replacement(u32);
impl Source for Replacement {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some([0, 0, 1024, 128].into()),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::Neutral,
        }
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(("callable replacement", self.0)))
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        Ok(vec![
            [100., 200., 300., 0.99];
            region.width as usize * region.height as usize
        ])
    }
}
struct Tint([f32; 4]);
impl Operation for Tint {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(("fixture tint", self.0)))
    }
    fn reads(&self, output: CellRegion) -> Result<Vec<Read>, Error> {
        Ok(vec![Read::new(0, output)])
    }
    fn calculation(
        self: std::sync::Arc<Self>,
        inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _budget: crate::gpu::evaluation::Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, crate::Error> {
        let inputs = crate::gpu::evaluation::value::Calculation::references(inputs);
        let function=Function::new(
            "@group(0) @binding(6) var<uniform> private_color:vec4f;\n fn private_tint(p:vec4f,at:vec2f)->vec4f {return vec4f(private_color.rgb,p.w);}",
            "private_tint"
        ).with_uniform(6, &self.0);
        value::Calculation::new(
            domain,
            output,
            vec![inputs[0].request(output)?],
            value::Operation::shader_function(self.key(), function),
        )
        .map(Some)
    }
}
#[test]
fn replacement_arguments_keep_private_captures_masks_and_original_metadata_separate() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let base = source(Arc::new(Plate(Arc::new(Mutex::new(Vec::new())))));
    let values = [[-2., 3., 0.125, 0.], [4., -0.25, 0.5, 0.]];
    let replacements = values
        .into_iter()
        .enumerate()
        .map(|(i, color)| {
            let input = source(Arc::new(Replacement(i as u32)));
            let tinted = Value::from_operation(input.domain(), [input], Tint(color));
            (repair([20 + i as u32 * 20, 24], [0, 0]).support, tinted)
        })
        .collect();
    let root = compose_images(base, replacements).unwrap();
    let region: CellRegion = [18, 22, 24, 5].into();
    let budget = Budget {
        tile: [24, 5],
        scratch_bytes: 1 << 20,
    };
    let query = context
        .request(
            root.clone(),
            Region {
                bounds: region,
                grid: root.domain().grid,
            },
            budget,
        )
        .unwrap();
    let mut actual = vec![[0.; 4]; 24 * 5];
    context
        .read(
            &queue,
            &Default::default(),
            &query,
            || false,
            |block, pixels| {
                for row in 0..block.height {
                    let from = row as usize * block.width as usize;
                    let to = region.index([block.x, block.y + row as i32]);
                    actual[to..to + block.width as usize]
                        .copy_from_slice(&pixels[from..from + block.width as usize]);
                }
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    for y in region.y..region.bottom() {
        for x in region.x..region.right() {
            let expected = match [x, y] {
                [20, 24] => [-2., 3., 0.125, 0.75],
                [40, 24] => [4., -0.25, 0.5, 0.75],
                _ => [2., -0.5, 0.25, 0.75],
            };
            assert_eq!(
                actual[region.index([x, y])],
                expected,
                "independent sibling colors must affect only their selected cell and never replace base metadata"
            );
        }
    }
}
