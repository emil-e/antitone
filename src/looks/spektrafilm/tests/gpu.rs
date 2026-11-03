use super::*;
use crate::looks::authoring;

#[test]
fn complete_native_treatment_and_final_bakes_match_upstream_in_the_authoring_view() {
    pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let reference = crate::looks::reference::ReferenceTransforms::new(&device, &queue)
            .await
            .unwrap();
        let gate = crate::gpu::SubmissionGate::default();
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
                Some(model.bake(authoring::DEFAULT_GRID, &cancel).unwrap())
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
        println!(
            "Three default129³ bakes against source, including tetrahedral resampling: maxRGB error {maximum}"
        );
        assert!(
            maximum < 0.01,
            "viewed final-bake approximation error {maximum}"
        );
    });
}
