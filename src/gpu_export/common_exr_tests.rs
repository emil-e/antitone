use super::*;
use crate::adjustment_layers::AdjustmentLayerPipeline;
use crate::processing::Calibration;
use crate::working_precision;
use exr::prelude::*;

#[test]
fn common_linear_engine_exr_matches_independent_scalar_with_shared_working_budget() {
    let (device, queue, renderer) = super::tests::context();
    let gate = SubmissionGate::default();
    let scan = Scan::open(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        None,
    )
    .unwrap();
    let calibration = Calibration {
        dmin: DVec3::splat(3000. / 4031.),
        dmax: DVec3::splat(128. / 4031.),
        input_floor: scan.input_floor,
    };
    let roll = RollSettings::default();
    let grade =
        AdjustmentLayerSnapshot::new(vec![crate::adjustment_layers::AdjustmentLayer::global(
            crate::adjustment_layers::AdjustmentLayerSettings {
                contrast: 0.8,
                saturation: 1.3,
                ..Default::default()
            },
        )])
        .unwrap();
    let scalar =
        AdjustmentLayerPipeline::new(calibration, roll, grade.clone(), scan.camera_to_ap1).unwrap();
    let a = scan.area;
    let exact = Composition::new(a, Default::default(), Default::default()).unwrap();
    let rotated = Composition::with_crop(
        a,
        Default::default(),
        Default::default(),
        Some(crate::crop::Crop {
            center: [
                f64::from(a.x) + f64::from(a.width) / 2.,
                f64::from(a.y) + f64::from(a.height) / 2.,
            ],
            size: [a.width - 64, a.height - 64],
            angle: 0.13,
        }),
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let mut violations = 0;
    for (name, composition) in [("exact", exact), ("rotated", rotated)] {
        for precision in [ExrPrecision::Half, ExrPrecision::Full] {
            let actual = directory
                .path()
                .join(format!("common-{name}-{precision:?}.exr"));
            let expected = directory
                .path()
                .join(format!("reference-{name}-{precision:?}.exr"));
            super::write(
                &renderer,
                &device,
                (&queue, &gate),
                Request {
                    film_emulation: Default::default(),
                    film_seed: 17,
                    scan: &scan,
                    field: None,
                    input: calibration.into(),
                    roll,
                    grade: grade.clone(),
                    repairs: &[],
                    composition,
                    format: Format::Exr(precision),
                    look: None,
                },
                &actual,
                Overwrite::Refuse,
            )
            .unwrap();
            crate::export::exr_composed(
                &expected,
                composition,
                precision,
                Overwrite::Refuse,
                |x, y| scalar.apply(scan.pixel(x, y)?, [x, y]).map(|p| p.rgb),
            )
            .unwrap();
            let actual = read_first_flat_layer_from_file(actual).unwrap();
            let expected = read_first_flat_layer_from_file(expected).unwrap();
            assert_eq!(actual.layer_data.size, expected.layer_data.size);
            let mut failed = 0;
            let mut maximum = 0_f64;
            let mut witness = None;
            for (actual, expected) in actual
                .layer_data
                .channel_data
                .list
                .iter()
                .zip(&expected.layer_data.channel_data.list)
            {
                assert_eq!(actual.name, expected.name);
                let (values, reference, half): (Vec<f64>, Vec<f64>, bool) =
                    match (&actual.sample_data, &expected.sample_data) {
                        (FlatSamples::F16(a), FlatSamples::F16(b)) => (
                            a.iter().map(|v| v.to_f64()).collect(),
                            b.iter().map(|v| v.to_f64()).collect(),
                            true,
                        ),
                        (FlatSamples::F32(a), FlatSamples::F32(b)) => (
                            a.iter().map(|&v| f64::from(v)).collect(),
                            b.iter().map(|&v| f64::from(v)).collect(),
                            false,
                        ),
                        _ => panic!("wrong EXR sample precision"),
                    };
                assert_eq!(values.len(), reference.len());
                assert!(reference.iter().all(|v| v.is_finite()));
                let min = reference.iter().copied().fold(f64::INFINITY, f64::min);
                let max = reference.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                assert!(
                    max - min > 0.01,
                    "each reference channel must have spatial variation"
                );
                for (i, (&value, &expected)) in values.iter().zip(&reference).enumerate() {
                    assert!(value.is_finite());
                    let error = (value - expected).abs();
                    maximum = maximum.max(error);
                    // Common FP32 working/operator comparison budget, plus storage rounding.
                    // Signed cancellation and constant conservation have separate exact witnesses;
                    // the container's mantissa does not set the internal processing contract.
                    let quantization = if half {
                        0.0009765625 * expected.abs() + 5.960464477539063e-8
                    } else {
                        f64::from(f32::EPSILON) * expected.abs()
                    };
                    let allowed = working_precision::allowance(expected) + quantization;
                    if error > allowed {
                        failed += 1;
                        witness.get_or_insert((
                            actual.name.clone(),
                            i,
                            value,
                            expected,
                            error,
                            allowed,
                        ));
                    }
                }
            }
            println!(
                "common_exr_probe geometry={name} precision={precision:?} violations={failed} max_absolute={maximum:.12e} witness={witness:?}"
            );
            violations += failed;
        }
    }
    assert_eq!(
        violations, 0,
        "both EXR storage types use the same independently checked FP32 working engine"
    );
}
