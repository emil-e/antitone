use super::*;
use crate::{
    adjustment_layers::{AdjustmentLayer, AdjustmentLayerSettings, AdjustmentLayerSnapshot},
    color::Balance,
    gpu::{GpuInputProgram, Grading},
    pixel_pipeline::{EffectDefinition, PixelStage},
    processing::{Calibration, InputSettings, NegativeInput, Response},
};
use glam::{DMat3, DVec3};

#[derive(Debug, Clone, Copy, PartialEq)]
struct Scale(DVec3);

impl EffectDefinition for Scale {
    fn support(&self, _: [f64; 2]) -> crate::processing_support::Support {
        crate::processing_support::Support::NONE
    }

    fn compose(&self, image: Arc<ImageNode>) -> Result<Arc<ImageNode>, Error> {
        let meaning = image.domain.meaning;
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

struct Pixels {
    domain: Domain,
    values: Vec<[f32; 4]>,
}

impl Source for Pixels {
    fn domain(&self) -> Domain {
        self.domain
    }

    fn read(&self, region: crate::geometry::SourceRegion) -> Result<Vec<[f32; 4]>, Error> {
        Ok((region.x..region.x + region.width)
            .map(|x| self.values[(x - self.domain.bounds.x) as usize])
            .collect())
    }
}

fn read(context: &Context, queue: &wgpu::Queue, image: Arc<ImageNode>) -> Vec<DVec3> {
    let request = context
        .request(
            image.clone(),
            Region {
                bounds: image.domain.bounds,
                grid: image.domain.grid,
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
            &crate::gpu::SubmissionGate::default(),
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
fn inserting_one_upstream_operation_changes_independent_prefixes_and_complete_compositions() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let context = renderer.evaluation();
    let area = crate::geometry::SourceRegion::from([21, 17, 4, 1]);
    let domain = Domain {
        bounds: area,
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
            input_floor: 1e-4,
        },
        gamma: 1.,
        balance: Balance::default(),
        response: Response {
            toe: 1.,
            shoulder: 0.,
        },
    };
    let matrix = DMat3::from_cols_array(&[1.2, 0.1, -0.1, 0.05, 0.9, 0.05, -0.1, 0.1, 1.1]);
    let program = GpuInputProgram::new(negative, matrix).unwrap();
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
    let grade = Arc::new(Grading::new(&device, roll, layers.clone(), area).unwrap());
    let captured = Captured::new(
        &device,
        &renderer.camera,
        CameraInput {
            matrix,
            matrix_buffer: None,
            field: None,
            continuous: true,
            mapping: Mapping::Originals,
        },
        Some(program.clone()),
        None,
    )
    .unwrap();
    let mut assembly = Assembly::captured(captured);
    assembly.grading = Some(grade.for_grid(0));
    assembly.photographic = Some(Arc::new(Photographic {
        parameters: ProcessingParameters::interpret(negative, roll, matrix).unwrap(),
        grade: Some(grade),
        repairs: Default::default(),
    }));
    // Gamma-one, unshaped inversion reduces to a ratio, and the two exposures give 2 × ¼.
    // This fixture checks operation placement without a second photographic evaluator.
    let native_response = |sensor: DVec3| {
        let calibration = negative.calibration;
        (calibration.dmin / sensor - DVec3::ONE)
            / (calibration.dmin / calibration.dmax - DVec3::ONE)
    };
    let grade_gain = 0.5;
    let scale = Scale(DVec3::new(0.75, 1.25, 0.9));
    let effect = PixelPipeline {
        stages: vec![PixelStage::new(scale)],
    };

    for before in [Boundary::Native, Boundary::Roll] {
        let mut steps = definition(&PixelPipeline::default());
        let index = steps
            .iter()
            .position(|step| step.boundary == Some(before))
            .unwrap();
        steps.insert(
            index,
            Step {
                boundary: None,
                component: Component::Effects(effect.clone()),
            },
        );

        let full = assembly
            .clone()
            .build(codes.clone(), Boundary::Codes, Boundary::Processed, &steps)
            .unwrap();
        let prefix = assembly
            .clone()
            .build(codes.clone(), Boundary::Codes, Boundary::Neutral, &steps)
            .unwrap();
        let native = Assembly::native(program.clone(), RollSettings::default())
            .build(
                normalized.clone(),
                Boundary::Camera,
                Boundary::Native,
                &steps,
            )
            .unwrap();
        let actual = read(context, &queue, full.at(Boundary::Processed));
        let neutral = read(context, &queue, prefix.at(Boundary::Neutral));
        let native = read(context, &queue, native.at(Boundary::Native));
        let mut sensitivity: f64 = 0.;

        for (index, input) in camera.into_iter().enumerate() {
            let interpreted_input = if before == Boundary::Native {
                input * scale.0
            } else {
                input
            };
            let working = matrix * native_response(interpreted_input);
            let grade_input = if before == Boundary::Roll {
                working * scale.0
            } else {
                working
            };
            let expected = grade_input * grade_gain;
            close(actual[index], expected);
            close(neutral[index], working);
            close(native[index], native_response(interpreted_input));
            sensitivity = sensitivity.max(
                (expected - matrix * native_response(input) * grade_gain)
                    .abs()
                    .max_element(),
            );
        }
        assert!(
            sensitivity > 0.05,
            "the fixture must detect an omitted operation"
        );

        if before == Boundary::Roll {
            let filtered = Assembly::filtered(assembly.grading.as_ref().unwrap().clone())
                .build(
                    prefix.at(Boundary::Neutral),
                    Boundary::Neutral,
                    Boundary::Processed,
                    &steps,
                )
                .unwrap();
            let filtered = read(context, &queue, filtered.at(Boundary::Processed));
            for (filtered, actual) in filtered.into_iter().zip(actual) {
                close(filtered, actual);
            }
        }
    }
}

#[test]
fn encoded_readouts_stop_at_the_requested_boundary_and_reject_unproven_scaling_crossings() {
    let negative = InputSettings::negative(Calibration {
        dmin: DVec3::splat(0.7),
        dmax: DVec3::splat(0.14),
        input_floor: 1e-4,
    });
    let program = GpuInputProgram::new(negative, DMat3::IDENTITY).unwrap();
    let encoding = RollSettings {
        exposure: -crate::processing::MAX_EXPOSURE,
        ..Default::default()
    };
    let assembly = Assembly::native(program, encoding);
    let image = source(Arc::new(Pixels {
        domain: Domain {
            bounds: [0, 0, 1, 1].into(),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::Camera,
        },
        values: vec![[0.25, 0.25, 0.25, 0.]],
    }));
    let mut steps = definition(&PixelPipeline::default());
    let index = steps
        .iter()
        .position(|step| step.boundary == Some(Boundary::Unrepaired))
        .unwrap();
    steps.insert(
        index,
        Step {
            boundary: None,
            component: Component::Effects(PixelPipeline {
                stages: vec![PixelStage::new(Scale(DVec3::splat(0.5)))],
            }),
        },
    );

    assert!(
        assembly
            .clone()
            .build(image.clone(), Boundary::Camera, Boundary::Native, &steps)
            .is_ok()
    );
    assert!(matches!(
        assembly.build(image, Boundary::Camera, Boundary::Unrepaired, &steps),
        Err(Error::InvalidParameter(
            "readout encoding cannot cross an added image operation"
        ))
    ));
}
