use super::*;
use crate::adjustment_layers::AdjustmentLayer;
use crate::adjustment_layers::AdjustmentLayerSettings;
use crate::adjustment_layers::AdjustmentLayerSnapshot;
use crate::color::Balance;
use crate::gpu::grading::Grading;
use crate::gpu::interpretation::InputTransform;
use crate::pixel_pipeline::EffectDefinition;
use crate::pixel_pipeline::PixelStage;
use crate::processing::Calibration;
use crate::processing::InputSettings;
use crate::processing::NegativeInput;
use crate::processing::{Response, RollSettings};
use glam::{DMat3, DVec3};

#[test]
fn native_and_working_identity_use_captured_values_not_uniform_packing() {
    let input = InputSettings::Negative {
        dmin: Some(DVec3::splat(0.8)),
        dmax: Some(DVec3::splat(0.02)),
        response: Response::film(),
        gamma: 0.75,
        balance: Default::default(),
    };
    let capture =
        |input| Interpretation::Program(InputTransform::new(input, DMat3::IDENTITY).unwrap());
    let original = capture(input);
    let mut changed = input;
    if let InputSettings::Negative { gamma, .. } = &mut changed {
        *gamma = gamma.next_up();
    }
    let a = Native::new(original).unwrap();
    let b = Native::new(capture(changed)).unwrap();
    assert!(
        a.function().bindings == b.function().bindings,
        "control must collide in packed native constants"
    );
    assert_ne!(a.key(), b.key());
    assert_eq!(a.key(), Native::new(original).unwrap().key());

    let matrix = DMat3::IDENTITY;
    let mut changed = matrix;
    changed.x_axis.x = changed.x_axis.x.next_up();
    let a = Working::new(original, matrix).unwrap();
    let b = Working::new(original, changed).unwrap();
    assert!(
        a.function().bindings == b.function().bindings,
        "control must collide in packed camera constants"
    );
    assert_ne!(a.key(), b.key());
    assert_eq!(a.key(), Working::new(original, matrix).unwrap().key());
}

#[test]
fn repair_dag_provenance_is_described_without_pixels_and_rejects_mixed_interpretations() {
    use crate::dust_repair::{Repair, RepairId, RepairOrigin, SupportMask};
    use crate::gpu::{evaluation as e, photograph::PhotographGraph, repairs::definition};
    struct Unread;
    impl e::Source for Unread {
        fn domain(&self) -> e::Domain {
            e::Domain {
                bounds: Some(e::CellRegion::new([0, 0], [128, 128])),
                grid: e::Grid::axis_aligned([0.; 2], [1.; 2]),
                meaning: e::Meaning::CameraCodes,
            }
        }
        fn read(&self, _: e::CellRegion) -> Result<Vec<[f32; 4]>, Error> {
            panic!("provenance inspection must not acquire pixels")
        }
    }
    let source = e::source(Arc::new(Unread));
    let program = InputTransform::new(InputSettings::Digital, DMat3::IDENTITY).unwrap();
    let neutral = |program: InputTransform| {
        Assembly::captured(
            Captured::new(
                CameraInput {
                    matrix: program.matrix,
                    field: None,
                    continuous: false,
                    mapping: Mapping::Originals,
                },
                Interpretation::Program(program),
            )
            .unwrap(),
        )
        .unrepaired(source.clone())
        .unwrap()
    };
    let base = neutral(program);
    let support = SupportMask::new([30, 30], [3, 3], vec![true; 9]).unwrap();
    let repaired = definition::compose(
        base.clone(),
        &[Repair {
            id: RepairId(1),
            enabled: true,
            origin: RepairOrigin::Manual,
            support: support.clone(),
            offset: [32, 0],
        }],
    )
    .unwrap();
    assert_eq!(
        repaired.inputs().len(),
        2,
        "the control must not be a unary graph"
    );
    assert_eq!(Assembly::input_transform(&repaired).unwrap(), Some(program));
    assert!(PhotographGraph::has_correction(&repaired, None));
    let changed = InputTransform::new(InputSettings::Digital, DMat3::IDENTITY * 2.).unwrap();
    let foreign_heal = definition::healing_image(
        &neutral(changed),
        &Repair {
            id: RepairId(2),
            enabled: true,
            origin: RepairOrigin::Manual,
            support: support.clone(),
            offset: [32, 0],
        },
    )
    .unwrap();
    assert!(
        PhotographGraph::has_correction(&foreign_heal, None),
        "lowered CPU dependencies must still describe their correction choice without acquiring pixels"
    );
    let foreign_repair =
        definition::compose_images(base.clone(), vec![(support.clone(), foreign_heal)]).unwrap();
    assert!(
        Assembly::input_transform(&foreign_repair).is_err(),
        "a lowered whole CPU value must retain its upstream interpretation facts"
    );
    let conflicting = definition::compose_images(base, vec![(support, neutral(changed))]).unwrap();
    assert!(
        Assembly::input_transform(&conflicting).is_err(),
        "one photographic input must not claim conflicting branch interpretations"
    );
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Scale(DVec3);

impl EffectDefinition for Scale {
    fn compose(&self, image: Value) -> Result<Value, Error> {
        let meaning = image.domain().meaning;
        Ok(pointwise(
            image,
            meaning,
            Function::new(
                r#"
@group(0) @binding(18)
var<uniform> assembly_scale: vec4f;

fn scale_assembly_input(pixel: vec4f, position: vec2f) -> vec4f {
    return vec4f(pixel.rgb * assembly_scale.rgb, pixel.a);
}
"#,
                "scale_assembly_input",
            )
            .with_uniform(18, &self.0.as_vec3().extend(0.)),
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Square;

impl EffectDefinition for Square {
    fn compose(&self, image: Value) -> Result<Value, Error> {
        let meaning = image.domain().meaning;
        Ok(pointwise(
            image,
            meaning,
            Function::new(
                "fn square_assembly_input(pixel: vec4f, position: vec2f) -> vec4f { return vec4f(pixel.rgb * pixel.rgb, pixel.a); }",
                "square_assembly_input",
            ),
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct FootprintScale;

impl EffectDefinition for FootprintScale {
    fn integrates_footprint(&self) -> bool {
        true
    }

    fn compose(&self, image: Value) -> Result<Value, Error> {
        Scale(DVec3::splat(image.domain().grid.pitch[0])).compose(image)
    }
}

struct Pixels {
    domain: Domain,
    values: Vec<[f32; 4]>,
}

impl Source for Pixels {
    fn domain(&self) -> Domain {
        self.domain
    }

    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        Ok((region.x..region.right())
            .map(|x| self.values[(x - self.domain.finite().unwrap().x) as usize])
            .collect())
    }
}

fn read(context: &Context, queue: &wgpu::Queue, image: Value) -> Vec<DVec3> {
    let request = context
        .request(
            image.clone(),
            Region {
                bounds: image.domain().finite().unwrap(),
                grid: image.domain().grid,
            },
            Budget {
                tile: [2, 1],
                scratch_bytes: 1 << 20,
            },
        )
        .unwrap();
    let mut result = Vec::new();
    context
        .read(
            queue,
            &crate::gpu::submission::SubmissionGate::default(),
            &request,
            || false,
            |_, pixels| {
                for pixel in pixels {
                    assert_eq!(
                        pixel[3], 0.,
                        "assembly must preserve interpretation metadata"
                    );
                    result.push(DVec3::new(
                        pixel[0] as f64,
                        pixel[1] as f64,
                        pixel[2] as f64,
                    ));
                }
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    result
}

fn close(actual: DVec3, expected: DVec3) {
    assert!(
        (actual - expected).abs().max_element() < 2e-5 * expected.abs().max_element().max(1.),
        "{actual:?} != {expected:?}"
    );
}

#[test]
fn original_endpoints_preserve_interpretation_and_order_post_grade_operations() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let context = renderer.evaluation();
    let area = crate::geometry::SourceRegion::from([21, 17, 4, 1]);
    let domain = Domain {
        bounds: Some(area.try_into().unwrap()),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::CameraCodes,
    };
    let camera = [
        DVec3::new(0.125, 0.25, 0.5),
        DVec3::new(1., 0.5, 0.125),
        DVec3::new(1. / 64., 1. / 128., 1. / 32.),
        DVec3::new(1.5, 1.25, 1.),
    ];
    let codes = source(Arc::new(Pixels {
        domain,
        values: camera
            .iter()
            .map(|p| (*p * 65535.).as_vec3().extend(0.).to_array())
            .collect(),
    }));
    let normalized = source(Arc::new(Pixels {
        domain: Domain {
            meaning: Meaning::Camera,
            ..domain
        },
        values: camera
            .iter()
            .map(|p| p.as_vec3().extend(0.).to_array())
            .collect(),
    }));
    let negative = NegativeInput {
        calibration: Calibration {
            dmin: DVec3::new(0.7, 0.6, 0.5),
            dmax: DVec3::new(0.1, 0.08, 0.06),
        },
        gamma: 1.,
        balance: Balance::default(),
        response: Response {
            toe: 1.,
            shoulder: 0.,
        },
    };
    let matrix = DMat3::from_cols_array(&[1.2, 0.1, -0.1, 0.05, 0.9, 0.05, -0.1, 0.1, 1.1]);
    let program = InputTransform::new(negative, matrix).unwrap();
    let roll = RollSettings {
        exposure: 1.,
        ..Default::default()
    };
    let layers =
        AdjustmentLayerSnapshot::new(vec![AdjustmentLayer::global(AdjustmentLayerSettings {
            exposure: -2.,
            ..Default::default()
        })])
        .unwrap();
    let grade = Arc::new(Grading::new(&device.limits(), roll, layers.clone(), area).unwrap());
    let captured = Captured::new(
        CameraInput {
            matrix,
            field: None,
            continuous: true,
            mapping: Mapping::Originals,
        },
        Interpretation::Program(program),
    )
    .unwrap();
    let mut assembly = Assembly::captured(captured);
    assembly.grading = Some(grade.for_grid(0));

    // Gamma-one, unshaped inversion reduces to a ratio, and the two exposures give 2 × ¼.
    // This fixture checks operation placement without a second photographic evaluator.
    let native_response = |sensor: DVec3| {
        let calibration = negative.calibration;
        (calibration.dmin / sensor - DVec3::ONE)
            / (calibration.dmin / calibration.dmax - DVec3::ONE)
    };
    let grade_gain = 0.5;
    let scale = Scale(DVec3::new(0.75, 1.25, 0.9));
    let effects = PixelPipeline {
        stages: vec![PixelStage::new(Square), PixelStage::new(scale)],
    };
    let graph = assembly
        .clone()
        .photograph(codes.clone(), &effects)
        .unwrap();
    let prefix = assembly.neutral(codes).unwrap();
    let native = Assembly::native(program).interpret(normalized).unwrap();
    let actual = read(context, &queue, graph.processed);
    let neutral = read(context, &queue, prefix);
    let native = read(context, &queue, native);
    let mut sensitivity: f64 = 0.;
    for (index, input) in camera.into_iter().enumerate() {
        let working = matrix * native_response(input);
        let graded = working * grade_gain;
        let expected = graded * graded * scale.0;
        close(actual[index], expected);
        close(neutral[index], working);
        close(native[index], native_response(input));
        sensitivity = sensitivity.max(
            (expected - working * working * grade_gain * scale.0)
                .abs()
                .max_element(),
        );
    }
    assert!(
        sensitivity > 0.05,
        "the fixture must detect squaring before the grade"
    );
}

#[test]
fn filtered_output_keeps_upstream_components_and_delays_footprint_effects_until_presentation() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let domain = Domain {
        bounds: Some([0, 0, 4, 1].into()),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Neutral,
    };
    let values = [0.125, 0.5, 1., 4.];
    let input = source(Arc::new(Pixels {
        domain,
        values: values.map(|v| [v, v, v, 0.]).to_vec(),
    }));
    let grading = Grading::new(
        &device.limits(),
        RollSettings {
            exposure: 1.,
            ..Default::default()
        },
        AdjustmentLayerSnapshot::new(vec![AdjustmentLayer::global(AdjustmentLayerSettings {
            exposure: -2.,
            ..Default::default()
        })])
        .unwrap(),
        domain.finite().unwrap().source().unwrap(),
    )
    .unwrap();
    let effects = PixelPipeline {
        stages: vec![PixelStage::new(FootprintScale)],
    };
    let upstream = DVec3::new(0.75, 1.25, 0.9);
    let input = Square.compose(input).unwrap();
    let input = Scale(upstream).compose(input).unwrap();
    let assembly = Assembly::filtered(grading);
    let mut output = domain;
    output.meaning = Meaning::Graded;
    output.bounds = Some([0, 0, 2, 1].into());
    output.grid = Grid::axis_aligned([0.; 2], [2., 1.]);
    let (_, graded) = assembly.grade(input.clone()).unwrap();
    assert!(
        graded.domain().same_grid(domain),
        "grading retains its supplied native grid"
    );
    let prefix = read(renderer.evaluation(), &queue, graded);
    for (value, original) in prefix.into_iter().zip(values) {
        close(value, upstream * f64::from(original).powi(2) * 0.5);
    }
    let image = assembly
        .filtered_image(input, &effects, output, [0; 2], [2., 1.])
        .unwrap();
    assert!(image.domain().same_grid(output));
    let actual = read(renderer.evaluation(), &queue, image);
    for (pair, actual) in values.as_chunks::<2>().0.iter().zip(actual) {
        // Squaring precedes averaging, so the result contains the mean of squares,
        // not the square of the mean. Roll/layer gain is 2 × 1/4; the final
        // footprint-aware effect sees pitch2, rather than native pitch1.
        close(
            actual,
            upstream * (f64::from(pair[0]).powi(2) + f64::from(pair[1]).powi(2)) * 0.5,
        );
    }
}
