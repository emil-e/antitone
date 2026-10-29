use super::*;
use crate::{
    adjustment_layers::AdjustmentLayerStack,
    geometry::{SourcePoint, SourceRegion},
    gpu::{GpuFlatField, GpuPipeline, ProcessingInput},
    processing::{Calibration, InputPipeline, InputSettings, RollPipeline},
};
use glam::DVec3;

struct Codes {
    area: SourceRegion,
    pixels: Vec<[f32; 4]>,
}

impl Source for Codes {
    fn domain(&self) -> Domain {
        Domain {
            bounds: self.area,
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::CameraCodes,
        }
    }

    fn read(&self, r: SourceRegion) -> Result<Vec<[f32; 4]>, Error> {
        Ok((r.y..r.y + r.height)
            .flat_map(|y| {
                (r.x..r.x + r.width).map(move |x| {
                    self.pixels[((y - self.area.y) * self.area.width + x - self.area.x) as usize]
                })
            })
            .collect())
    }
}

fn evaluate(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &crate::renderer::GpuRenderer,
    root: Arc<ImageNode>,
    region: SourceRegion,
    policy: Policy,
) -> (Vec<[f32; 4]>, usize) {
    let plan = Plan::with_policy(
        root,
        region,
        Budget {
            tile: [17, 13],
            scratch_bytes: 8 << 20,
        },
        policy,
    )
    .unwrap();
    let computes = plan
        .tiles
        .iter()
        .flat_map(|t| &t.fragment.work)
        .filter(|w| matches!(w, Work::Compute { .. }))
        .count();
    let pixels = Executor::with_programs(renderer.evaluation.programs.clone())
        .run(device, queue, &Default::default(), &plan, || false)
        .unwrap()
        .unwrap();
    (execute::read(device, queue, &pixels.pixels), computes)
}

#[test]
fn camera_neutral_repair_roll_and_stack_are_requestable_images_with_a_fused_original_implementation()
 {
    use crate::dust_repair::{Repair, RepairId, RepairOrigin, SupportMask};
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let area = SourceRegion::from([11, 17, 81, 67]);
    let pixels: Vec<_> = (0..area.width * area.height)
        .map(|i| {
            let v = [0., 0.003, 0.035, 0.18, 0.7, 1.5][i as usize % 6];
            [
                (v * 65535.) as f32,
                ((v * 0.7 + 0.004) * 65535.) as f32,
                ((v * 0.5 + 0.012) * 65535.) as f32,
                0.,
            ]
        })
        .collect();
    let codes = Arc::new(Codes { area, pixels });
    let field = Arc::new(
        crate::flat_field::FlatField::from_nodes(
            [area.x, area.y, area.width, area.height],
            [2, 2],
            vec![
                DVec3::new(1.2, 0.9, 1.1),
                DVec3::new(0.95, 1.1, 0.85),
                DVec3::new(1.1, 1.3, 0.95),
                DVec3::new(0.9, 1.05, 1.2),
            ],
        )
        .unwrap(),
    );
    let gpu_field = Arc::new(GpuFlatField::new(&device, field.clone()).unwrap());
    let repair = Repair {
        id: RepairId(1),
        enabled: true,
        origin: RepairOrigin::Manual,
        support: SupportMask::new([area.x + 20, area.y + 17], [12, 10], vec![true; 120]).unwrap(),
        offset: [25, 18],
    };
    let repaired =
        Arc::new(crate::dust_repair::evaluate(&repair, area, |_| Ok([-0.4, 2.6, 0.7])).unwrap());
    let snapshot = crate::gpu::repairs::Snapshot::logical(vec![repaired.clone()]).unwrap();
    let region = SourceRegion::from([area.x + 7, area.y + 9, 51, 35]);
    let matrix = super::tests::matrix();
    let (roll, stack) = super::tests::settings(area);
    let scalar_roll = RollPipeline::new(roll).unwrap();
    let scalar_stack = AdjustmentLayerStack::new(stack.layers().to_vec()).unwrap();
    for settings in [
        InputSettings::Digital,
        InputSettings::negative(Calibration {
            dmin: DVec3::splat(0.7),
            dmax: DVec3::splat(0.14),
            // Bound ordinary RGB for the absolute-error gate while exercising below-floor flags.
            input_floor: 0.1,
        }),
    ] {
        let pipeline = Arc::new(
            GpuPipeline::new(&device, settings, roll, stack.clone(), matrix, area).unwrap(),
        );
        let scalar_input = InputPipeline::new(settings, matrix).unwrap();
        let mut input = ProcessingInput::from(pipeline);
        input.repairs = snapshot.clone();
        let photo = Photograph::new(
            &device,
            &renderer,
            source(codes.clone()),
            Some(gpu_field.clone()),
            &input,
            matrix,
        )
        .unwrap();
        let camera = evaluate(
            &device,
            &queue,
            &renderer,
            photo.camera.clone(),
            region,
            Policy::Composed,
        )
        .0;
        assert_eq!(camera, codes.read(region).unwrap());
        let mut expected = vec![Vec::new(); 4];
        let mut floor_flags = false;
        let mut signed_hdr = [false; 2];
        let mut repair_effect = 0_f64;
        let mut grade_effect = 0_f64;
        let mut correction_effect = 0_f64;
        for (i, raw) in camera.iter().enumerate() {
            let point = [
                region.x + i as u32 % region.width,
                region.y + i as u32 / region.width,
            ];
            let position = SourcePoint(point.map(|v| f64::from(v) + 0.5));
            let raw = DVec3::new(raw[0] as f64, raw[1] as f64, raw[2] as f64) / 65535.;
            let base = scalar_input
                .apply(raw * field.gain(position.0).unwrap())
                .unwrap();
            let flags = base
                .floored
                .iter()
                .enumerate()
                .fold(0u32, |v, (i, on)| v | ((*on as u32) << i));
            floor_flags |= flags != 0;
            correction_effect = correction_effect.max(
                (base.rgb - scalar_input.apply(raw).unwrap().rgb)
                    .abs()
                    .max_element(),
            );
            let neutral = DVec3::from_array(
                repaired
                    .pixel(point, base.rgb.to_array().map(|v| v as f32))
                    .map(f64::from),
            );
            repair_effect = repair_effect.max((neutral - base.rgb).abs().max_element());
            signed_hdr[0] |= neutral.min_element() < 0.;
            signed_hdr[1] |= neutral.max_element() > 1.;
            let roll = scalar_roll.apply(neutral).unwrap();
            let graded = scalar_stack.apply_rgb(roll, position).unwrap();
            grade_effect = grade_effect.max((graded - roll).abs().max_element());
            for (output, rgb) in expected.iter_mut().zip([base.rgb, neutral, roll, graded]) {
                output.push([rgb.x, rgb.y, rgb.z, flags as f64]);
            }
        }
        assert_eq!(signed_hdr, [true, true]);
        assert!(
            repair_effect > 0.01 && grade_effect > 0.01 && correction_effect > 0.01,
            "fixture must distinguish repairs, grading and correction"
        );
        assert_eq!(
            floor_flags,
            matches!(settings, InputSettings::Negative { .. })
        );
        let mut work = Vec::new();
        for (root, expected) in [
            photo.unrepaired.clone(),
            photo.neutral.clone(),
            photo.roll.clone(),
            photo.graded.clone(),
        ]
        .into_iter()
        .zip(expected)
        {
            let mut counts = Vec::new();
            for policy in [Policy::Materialized, Policy::Composed] {
                let (actual, count) =
                    evaluate(&device, &queue, &renderer, root.clone(), region, policy);
                counts.push(count);
                assert_eq!(actual.len(), expected.len());
                for (actual, expected) in actual.iter().zip(&expected) {
                    assert!(actual.iter().all(|v| v.is_finite()));
                    assert_eq!(actual[3] as f64, expected[3]);
                    for c in 0..3 {
                        assert!(
                            (actual[c] as f64 - expected[c]).abs() < 2e-5,
                            "{settings:?}: {actual:?} vs {expected:?}"
                        );
                    }
                }
            }
            work.push(counts);
        }
        assert!(
            work[3][1] < work[3][0],
            "addressable photographic boundaries need not add original processing passes"
        );
        let grading = Grading::new(&device, input.grade().unwrap(), 0);
        let doubled = pointwise(
            photo.neutral.clone(),
            Meaning::Neutral,
            Function {
                code: r#"
fn double(p: vec4f, pos: vec2f) -> vec4f {
    return vec4f(p.rgb * 2., p.w);
}
"#
                .into(),
                symbol: "double",
                bindings: vec![],
            },
        );
        let changed = grading.compose(doubled).unwrap();
        let actual = evaluate(
            &device,
            &queue,
            &renderer,
            changed,
            region,
            Policy::Composed,
        )
        .0;
        let neutral = evaluate(
            &device,
            &queue,
            &renderer,
            photo.neutral,
            region,
            Policy::Composed,
        )
        .0;
        let mut insertion_effect = 0_f64;
        let original = evaluate(
            &device,
            &queue,
            &renderer,
            photo.graded,
            region,
            Policy::Composed,
        )
        .0;
        for (i, (actual, neutral)) in actual.iter().zip(neutral).enumerate() {
            let position = SourcePoint([
                f64::from(region.x + i as u32 % region.width) + 0.5,
                f64::from(region.y + i as u32 / region.width) + 0.5,
            ]);
            let rgb = DVec3::new(neutral[0] as f64, neutral[1] as f64, neutral[2] as f64) * 2.;
            let expected = scalar_stack
                .apply_rgb(scalar_roll.apply(rgb).unwrap(), position)
                .unwrap();
            let actual = DVec3::new(actual[0] as f64, actual[1] as f64, actual[2] as f64);
            assert!((actual - expected).abs().max_element() < 2e-5);
            insertion_effect = insertion_effect.max(
                (actual
                    - DVec3::new(
                        original[i][0] as f64,
                        original[i][1] as f64,
                        original[i][2] as f64,
                    ))
                .abs()
                .max_element(),
            );
        }
        assert!(
            insertion_effect > 0.01,
            "an inserted component cannot be skipped by the original-cell implementation"
        );
    }
}

#[test]
fn folded_negative_original_does_not_require_a_representable_neutral_intermediate() {
    use crate::{
        adjustment_layers::AdjustmentLayerSnapshot,
        processing::{Pipeline, RollSettings},
    };
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let area = SourceRegion::from([11, 17, 3, 2]);
    let mut settings = InputSettings::negative(Calibration {
        dmin: DVec3::splat(0.7),
        dmax: DVec3::splat(0.14),
        input_floor: 1e-6,
    });
    if let InputSettings::Negative { gamma, .. } = &mut settings {
        *gamma = 0.125;
    }
    let roll = RollSettings {
        exposure: -16.,
        ..Default::default()
    };
    let matrix = glam::DMat3::IDENTITY;
    let pipeline = Arc::new(
        GpuPipeline::new(
            &device,
            settings,
            roll,
            AdjustmentLayerSnapshot::new(vec![]).unwrap(),
            matrix,
            area,
        )
        .unwrap(),
    );
    assert!(
        pipeline.input_program().neutral_parameters().is_err(),
        "fixture must require folded exposure to stay within f32"
    );
    let input = ProcessingInput::from(pipeline);
    let codes = source(Arc::new(Codes {
        area,
        pixels: vec![[0.; 4]; 6],
    }));
    let photo = Photograph::new(&device, &renderer, codes, None, &input, matrix).unwrap();
    let actual = evaluate(
        &device,
        &queue,
        &renderer,
        photo.graded,
        area,
        Policy::Composed,
    )
    .0;
    let expected = Pipeline::new(settings, roll, matrix)
        .unwrap()
        .apply(DVec3::ZERO)
        .unwrap()
        .rgb;
    assert!(expected.max_element() > f32::MAX as f64 / 65536.);
    for pixel in actual {
        assert!(pixel.iter().all(|v| v.is_finite()));
        assert_eq!(pixel[3], 7.);
        for c in 0..3 {
            assert!(
                (pixel[c] as f64 / expected[c] - 1.).abs() < 2e-5,
                "folded original {pixel:?} vs {expected:?}"
            );
        }
    }
    assert!(
        Plan::with_policy(
            photo.neutral,
            area,
            Budget {
                tile: [3, 2],
                scratch_bytes: 8 << 20
            },
            Policy::Composed
        )
        .is_err(),
        "an explicitly requested, unrepresentable intermediate must fail rather than change the photographic model"
    );
}

#[test]
fn equivalent_implementations_keep_the_domain_and_compose_with_downstream_functions() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Traced {
        function: Function,
        calls: Arc<AtomicUsize>,
    }

    impl Operation for Traced {
        fn input(&self, r: SourceRegion) -> Result<Option<SourceRegion>, Error> {
            Ok(Some(r))
        }

        fn pointwise(&self) -> Option<Function> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Some(self.function.clone())
        }

        fn lower(
            &self,
            _: &mut Fragment,
            _: Option<PlannedImage>,
            _: PlannedImage,
        ) -> Result<(), Error> {
            unreachable!("pointwise contract")
        }
    }
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let area = SourceRegion::from([11, 17, 3, 2]);
    let input = source(Arc::new(Codes {
        area,
        pixels: vec![[0.25, 0.5, 1.25, 0.]; 6],
    }));
    let function = |symbol, body: &str| Function {
        code: format!(
            r#"
fn {symbol}(p: vec4f, pos: vec2f) -> vec4f {{
    return vec4f({body}, p.w);
}}
"#
        )
        .into(),
        symbol,
        bindings: vec![],
    };
    let logical = pointwise(
        pointwise(
            input.clone(),
            Meaning::Neutral,
            function("twice", "p.rgb*2."),
        ),
        Meaning::Neutral,
        function("bias", "p.rgb+3."),
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let physical = ImageNode::new(
        logical.domain,
        Some(input),
        Traced {
            function: function("combined", "p.rgb*2.+vec3f(3.)"),
            calls: calls.clone(),
        },
    );
    for domain in [
        Domain {
            meaning: Meaning::Graded,
            ..logical.domain
        },
        Domain {
            bounds: [11, 17, 2, 2].into(),
            ..logical.domain
        },
        Domain {
            grid: Grid::axis_aligned([0.; 2], [2.; 2]),
            ..logical.domain
        },
    ] {
        let incompatible = ImageNode::new(
            domain,
            physical.input.clone(),
            Traced {
                function: function("combined", "p.rgb*2.+vec3f(3.)"),
                calls: calls.clone(),
            },
        );
        assert!(ImageNode::implemented_by(logical.clone(), incompatible).is_err());
    }
    let root = ImageNode::implemented_by(logical, physical).unwrap();
    let root = pointwise(root, Meaning::Neutral, function("after", "p.rgb*4."));
    let mut work = Vec::new();
    for policy in [Policy::Materialized, Policy::Composed] {
        calls.store(0, Ordering::Relaxed);
        let (actual, count) = evaluate(&device, &queue, &renderer, root.clone(), area, policy);
        assert_eq!(actual, vec![[14., 16., 22., 0.]; 6]);
        assert_eq!(
            calls.load(Ordering::Relaxed) > 0,
            policy == Policy::Composed,
            "only composed planning selects the physical implementation"
        );
        work.push(count);
    }
    assert!(
        work[1] <= work[0],
        "a supplied implementation does not force a downstream pointwise traversal"
    );
}

#[test]
fn keeping_a_neutral_image_does_not_retain_its_downstream_grade() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let area = SourceRegion::from([11, 17, 3, 2]);
    let pipeline = Arc::new(
        GpuPipeline::new(
            &device,
            InputSettings::Digital,
            Default::default(),
            crate::adjustment_layers::AdjustmentLayerSnapshot::new(vec![]).unwrap(),
            glam::DMat3::IDENTITY,
            area,
        )
        .unwrap(),
    );
    let grade = Arc::downgrade(&pipeline.grade);
    let input = ProcessingInput::from(pipeline);
    let pixels = vec![[0.25 * 65535., 0.5 * 65535., 1.5 * 65535., 0.]; 6];
    let photo = Photograph::new(
        &device,
        &renderer,
        source(Arc::new(Codes { area, pixels })),
        None,
        &input,
        glam::DMat3::IDENTITY,
    )
    .unwrap();
    let neutral = photo.neutral.clone();
    drop(photo);
    drop(input);
    assert!(
        grade.upgrade().is_none(),
        "a source-side image must not own the downstream photographic recipe"
    );
    let actual = evaluate(&device, &queue, &renderer, neutral, area, Policy::Composed).0;
    for pixel in actual {
        for (actual, expected) in pixel.into_iter().zip([0.25, 0.5, 1.5, 0.]) {
            assert!((actual - expected).abs() < 1e-6);
        }
    }
}
