use super::*;
#[cfg(feature = "ocio-reference")]
mod sampling;

#[test]
fn native_treatment_matches_source_and_reports_final_bake_approximation() {
    pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let reference = crate::looks::reference::ReferenceTransforms::new(&device, &queue)
            .await
            .unwrap();
        let gate = crate::gpu::submission::SubmissionGate::default();
        let cancel = AtomicBool::new(false);
        let f = fixtures();
        let cases = f
            .cases
            .into_iter()
            .map(|c| {
                (
                    Selection {
                        negative: Some(Negative::from_id(&c.negative).unwrap()),
                        paper: Paper::from_id(&c.paper).unwrap(),
                    },
                    c.input,
                    c.reference_rgb,
                )
            })
            .chain(f.paper_only.into_iter().map(|c| {
                (
                    Selection {
                        negative: None,
                        paper: Paper::from_id(&c.paper).unwrap(),
                    },
                    c.input,
                    c.reference_rgb,
                )
            }));
        let mut scene = Vec::new();
        let mut expected = Vec::new();
        let mut baked_scene = Vec::new();
        let mut baked_expected = Vec::new();
        let mut baked_count = 0;
        let mut contexts = Vec::new();
        let mut baked_contexts = Vec::new();
        for (selection, input, targets) in cases {
            let model = Model::new(selection, &cancel).unwrap();
            let representative_negative = selection
                .negative
                .is_none_or(|n| n.id() == "kodak_portra_400")
                || (selection.paper == Paper::default()
                    && selection
                        .negative
                        .is_some_and(|n| n.id() == "kodak_ektar_100"));
            let baked = if representative_negative {
                baked_count += 1;
                let asset = model
                    .bake(crate::looks::authoring::DEFAULT_GRID, &cancel)
                    .unwrap();
                crate::looks::sampling::assert_material_vertices(asset.lut(), |rgb| {
                    model.encoded(rgb)
                });
                Some(asset)
            } else {
                None
            };
            assert_eq!(input.len(), targets.len());
            for (input, target) in input.into_iter().zip(targets) {
                let rgb = model.encoded(input).unwrap().map(photographic::decode);
                let context = format!("{}: {input:?}", selection.name());
                contexts.push(context.clone());
                scene.push([rgb.x as f32, rgb.y as f32, rgb.z as f32, 1.]);
                expected.push(target);
                if let Some(asset) = &baked {
                    let rgb = asset.lut().apply(input).unwrap();
                    baked_scene.push([rgb.x as f32, rgb.y as f32, rgb.z as f32, 1.]);
                    baked_expected.push(target);
                    baked_contexts.push(context);
                }
            }
        }
        let errors = |actual: Vec<[f32; 4]>, expected: Vec<DVec3>, contexts: &[String]| {
            assert!(!actual.is_empty());
            assert_eq!(actual.len(), contexts.len());
            assert_eq!(actual.len(), expected.len());
            assert!(actual.iter().flatten().all(|v| v.is_finite()));
            let mut worst = (0., 0);
            for (i, (actual, expected)) in actual.iter().zip(&expected).enumerate() {
                let error = (DVec3::new(actual[0] as f64, actual[1] as f64, actual[2] as f64)
                    - expected)
                    .abs()
                    .max_element();
                if error > worst.0 {
                    worst = (error, i);
                }
            }
            println!(
                "Worst case {}: actual {:?}, source {:?}",
                contexts[worst.1], actual[worst.1], expected[worst.1]
            );
            worst.0
        };
        let count = expected.len();
        let actual = reference
            .apply(&device, &queue, &gate, false, &scene)
            .unwrap();
        let maximum = errors(actual, expected, &contexts);
        println!(
            "{count} executed-source cases, scanner/inverse/native component/reference-GPU maxRGB error {maximum}"
        );
        assert!(
            maximum < 0.01,
            "viewed component approximation error {maximum}"
        );
        assert_eq!(
            baked_count,
            Paper::all().len() * 2 + 1,
            "every medium must exercise paper-only and selected-negative production-size baking"
        );
        let actual = reference
            .apply(&device, &queue, &gate, false, &baked_scene)
            .unwrap();
        let maximum = errors(actual, baked_expected, &baked_contexts);
        // Approximation diagnostics are not perceptual acceptance thresholds;
        // judge ordinary gradients through the intended target DRT and encoding.
        println!(
            "{baked_count} provider-sized bakes against source, including tetrahedral resampling: maxRGB error {maximum}"
        );
        assert!(
            maximum < 0.01,
            "composed material/source approximation error {maximum}"
        );
    });
}
