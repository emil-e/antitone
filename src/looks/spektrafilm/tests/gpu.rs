use super::*;
use crate::looks::authoring;

#[test]
fn native_treatment_matches_upstream_and_reports_final_bake_approximation() {
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
        for (selection, input, targets) in cases {
            let model = Model::new(selection, &cancel).unwrap();
            let baked = if selection.paper == Paper::default()
                && selection
                    .negative
                    .is_none_or(|n| ["kodak_portra_400", "kodak_ektar_100"].contains(&n.id()))
            {
                baked_count += 1;
                let asset = model.bake(authoring::DEFAULT_GRID, &cancel).unwrap();
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
                scene.push([rgb.x as f32, rgb.y as f32, rgb.z as f32, 1.]);
                expected.push(target);
                if let Some(asset) = &baked {
                    let rgb = asset.lut().apply(input).unwrap();
                    baked_scene.push([rgb.x as f32, rgb.y as f32, rgb.z as f32, 1.]);
                    baked_expected.push(target);
                }
            }
        }
        let errors = |actual: Vec<[f32; 4]>, expected: Vec<DVec3>| {
            assert_eq!(actual.len(), expected.len());
            assert!(actual.iter().flatten().all(|v| v.is_finite()));
            actual
                .iter()
                .zip(&expected)
                .map(|(a, e)| {
                    (DVec3::new(a[0] as f64, a[1] as f64, a[2] as f64) - *e)
                        .abs()
                        .max_element()
                })
                .fold(0., f64::max)
        };
        let count = expected.len();
        let actual = reference
            .apply(&device, &queue, &gate, false, &scene)
            .unwrap();
        let maximum = errors(actual, expected);
        println!(
            "{count} executed-upstream cases, scanner/inverse/native component/reference-GPU maxRGB error {maximum}"
        );
        assert!(
            maximum < 0.01,
            "viewed component approximation error {maximum}"
        );
        assert_eq!(
            baked_count, 3,
            "both source stocks and paper-only must exercise production-size baking"
        );
        let actual = reference
            .apply(&device, &queue, &gate, false, &baked_scene)
            .unwrap();
        let maximum = errors(actual, baked_expected);
        // Approximation diagnostics are not perceptual acceptance thresholds;
        // judge ordinary gradients through the intended target DRT and encoding.
        println!(
            "Three default{}³ bakes against source, including tetrahedral resampling: maxRGB error {maximum}",
            authoring::DEFAULT_GRID
        );
    });
}
