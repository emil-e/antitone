use super::*;
use crate::{
    adjustment_layers::{AdjustmentLayer, AdjustmentLayerStack},
    processing::InvertedPixel,
};

#[test]
fn complete_ordered_grade_precedes_image_mtf_and_unblurred_grain() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let base = probe_view(&device, &queue, &renderer, &kernels, [1.8, 1.], true, 17);
        let source = base.graded.as_ref().unwrap().source.clone();
        let layers = vec![
            AdjustmentLayer::global(AdjustmentLayerSettings {
                exposure: 0.7,
                ..Default::default()
            }),
            AdjustmentLayer::global(AdjustmentLayerSettings {
                contrast: 1.4,
                saturation: 1.1,
                ..Default::default()
            }),
        ];
        let snapshot = AdjustmentLayerSnapshot::new(layers.clone()).unwrap();
        let scalar = AdjustmentLayerStack::new(layers).unwrap();
        let settings = FilmEmulationSettings {
            enabled: true,
            size: 1.8 / 6000. * 100.,
            amount: 0.055,
            colour: 0.5,
        };
        let input = ProcessingInput::from(Arc::new(
            GpuPipeline::new(
                &device,
                InputSettings::Digital,
                Default::default(),
                snapshot.clone(),
                DMat3::IDENTITY,
                source.spec.area,
            )
            .unwrap(),
        ))
        .with_film_emulation(settings)
        .with_frame_size(source.spec.frame().composition.crop().size)
        .with_film_seed(17);
        let view = RenderedView::new(&device, source, &input, OutputTarget::Srgb, None).unwrap();
        view.ensure_submitted(
            &device,
            &queue,
            &SubmissionGate::default(),
            &egui::Context::default(),
        );
        let graded = view.graded.as_ref().unwrap();
        let actual = read(&device, &queue, &graded.pixels);
        let field = read(&device, &queue, &graded.stages.field().unwrap().pixels);
        let field_size = graded.stages.raster.size();
        let offset = graded.plan.output_offset().unwrap();
        let grade = |rgb: [f32; 3]| {
            scalar
                .apply(
                    InvertedPixel {
                        rgb: DVec3::from_array(rgb.map(f64::from)),
                        floored: [false; 3],
                    },
                    crate::geometry::SourcePoint([1000.5; 2]),
                )
                .unwrap()
                .rgb
        };
        let flat = grade([0.18; 3]);
        let impulse = grade([0.18, 0.36, 0.18]);
        let sigma = 1.8_f64;
        let mass: f64 = (-8..=8)
            .map(|i| (-0.5 * f64::from(i * i) / (sigma * sigma)).exp())
            .sum();
        let variance = settings
            .parameters([6000; 2])
            .unwrap()
            .unwrap()
            .channel_variance(1.);
        let mut distinguishes_early_filter = false;
        for dy in -3..=3 {
            for dx in -3..=3 {
                let weight =
                    (-0.5 * f64::from(dx * dx + dy * dy) / (sigma * sigma)).exp() / (mass * mass);
                let expected_mtf = flat + weight * (impulse - flat);
                let early_filter = grade([0.18, 0.18 + 0.18 * weight as f32, 0.18]);
                distinguishes_early_filter |=
                    (expected_mtf - early_filter).abs().max_element() > 0.0001;
                let point = [(N as i32 / 2 + dx) as u32, (N as i32 / 2 + dy) as u32];
                let at = (point[1] * N as u32 + point[0]) as usize;
                let field_at =
                    ((point[1] + offset[1]) * field_size[0] + point[0] + offset[0]) as usize;
                for c in 0..3 {
                    let expected = expected_mtf[c]
                        * (settings.amount * f64::from(field[field_at][c])
                            - 0.5 * settings.amount.powi(2) * variance[c])
                            .exp();
                    assert!(
                        (f64::from(actual[at][c]) - expected).abs() < 0.000005,
                        "ordered grade → independent Gaussian → pointwise grain ({dx},{dy}), channel {c}"
                    );
                }
            }
        }
        assert!(
            distinguishes_early_filter,
            "fixture must distinguish MTF before the nonlinear finishing layer"
        );
    });
}
