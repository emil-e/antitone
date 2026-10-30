use super::*;
use crate::{
    gpu::evaluation::{self, ImageNode},
    pixel_pipeline::{EffectDefinition, PixelPipeline, PixelStage, test_effects::NeighbourEffect},
    processing_support::Support,
};

#[derive(Clone, Copy, Debug, PartialEq)]
struct Square;

impl EffectDefinition for Square {
    fn support(&self, _: [f64; 2]) -> Support {
        Support::NONE
    }

    fn view_pitch(&self) -> Option<f64> {
        Some(64.)
    }

    fn compose(&self, input: Arc<ImageNode>) -> Result<Arc<ImageNode>, Error> {
        let meaning = input.domain.meaning;
        Ok(evaluation::pointwise(
            input,
            meaning,
            evaluation::Function {
                code: "fn square(pixel: vec4f, position: vec2f) -> vec4f { return vec4f(pixel.rgb * pixel.rgb, pixel.w); }".into(),
                symbol: "square",
                bindings: vec![],
            },
        ))
    }
}

fn matches_native_arithmetic(actual: [f32; 3], expected: [f32; 3]) -> bool {
    (0..3).all(|c| {
        (f64::from(actual[c]) - f64::from(expected[c])).abs()
            <= 5e-5 * f64::from(expected[c]).abs().max(1.)
    })
}

#[test]
fn supplied_pipeline_uses_original_neighbours_before_rotated_output_reconstruction() {
    let (device, queue, renderer) = super::tests::context();
    let scan = Scan::open(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        None,
    )
    .unwrap();
    let area = scan.area;
    let composition = Composition::with_crop(
        area,
        Default::default(),
        Default::default(),
        Some(crate::crop::Crop {
            center: [
                f64::from(area.x) + f64::from(area.width) / 2.,
                f64::from(area.y) + f64::from(area.height) / 2.,
            ],
            size: [area.width - 64, area.height - 64],
            angle: 0.19,
        }),
    )
    .unwrap();
    let matrix = scan.positive_matrix(&InputSettings::Digital).unwrap();
    let at = |x: u32, y: u32| matrix * scan.pixel(x, y).unwrap();
    let bias =
        |x, y| DVec3::splat((f64::from(x) + 0.5) * 0.000001 + (f64::from(y) + 0.5) * 0.000002);
    let filtered = |x: u32, y: u32| {
        let left = x.saturating_sub(3).max(area.x);
        let right = (x + 3).min(area.x + area.width - 1);
        (at(left, y) + 2. * at(x, y) + at(right, y)) * 0.25 + bias(x, y)
    };
    let native = |x, y| {
        let value = filtered(x, y);
        value * value
    };
    let raster = FrameRaster::full(composition).unwrap();
    let [width, height] = composition.size();
    let ap0 = crate::color::ap1_to_ap0();
    let stored = |value: DVec3| (ap0 * value).as_vec3().to_array();
    let mut expected = Vec::new();
    let mut wrong_order_exposed = false;
    let mut omitted_neighbours_exposed = false;
    for y in 0..height {
        for x in 0..width {
            let cells: Vec<_> = raster
                .footprint([x, y])
                .unwrap()
                .pixels(area)
                .unwrap()
                .collect();
            let mean = |sample: &dyn Fn(u32, u32) -> DVec3| {
                crate::sampling::weighted_mean_iter(
                    cells
                        .iter()
                        .map(|(p, weight)| Ok((sample(p[0], p[1]), *weight))),
                )
                .unwrap()
            };
            let correct = stored(mean(&native));
            let before_square = mean(&filtered);
            wrong_order_exposed |=
                !matches_native_arithmetic(stored(before_square * before_square), correct);
            let center_only = mean(&|x, y| {
                let value = at(x, y) + bias(x, y);
                value * value
            });
            omitted_neighbours_exposed |= !matches_native_arithmetic(stored(center_only), correct);
            expected.push(correct);
        }
    }
    assert!(
        wrong_order_exposed,
        "processing averaged cells must fail the reopened AP0 comparison at its actual tolerance"
    );
    assert!(
        omitted_neighbours_exposed,
        "omitted neighbours must fail the reopened AP0 comparison at its actual tolerance"
    );

    let directory = tempfile::tempdir().unwrap();
    let actual = directory.path().join("pipeline.exr");
    let gate = SubmissionGate::default();
    write(
        &renderer,
        &device,
        (&queue, &gate),
        Request {
            scan: &scan,
            field: None,
            input: InputSettings::Digital,
            roll: Default::default(),
            grade: AdjustmentLayerSnapshot::new(vec![]).unwrap(),
            repairs: &[],
            composition,
            format: Format::Exr(ExrPrecision::Full),
            look: None,
            pipeline: PixelPipeline {
                stages: vec![
                    PixelStage::new(NeighbourEffect {
                        radius: 3.,
                        gain: 1.,
                    }),
                    PixelStage::new(Square),
                ],
            },
        },
        &actual,
        Overwrite::Refuse,
    )
    .unwrap();
    let actual = super::tests::exr_pixels(&actual);
    assert_eq!(actual.len(), (width * height) as usize);
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert!(
            matches_native_arithmetic(*actual, expected),
            "native pipeline and affine reconstruction: {actual:?} vs {expected:?}"
        );
    }
}
