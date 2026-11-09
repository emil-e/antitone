//! Optional approximation study using real, normally developed RAW samples.
//! Scalar input/roll evaluation supplies a photographic distribution, not a
//! pixel-exact oracle for the composed GPU photograph; local grades/texture omit.
use super::*;
use crate::gpu::submission::SubmissionGate;

#[test]
#[ignore = "requires an explicitly supplied private project and fresh report path"]
fn supplied_photo_distribution_measures_cube_approximation_without_retuning_materials() {
    let path = std::path::PathBuf::from(std::env::var_os("ANTITONE_BW_PROBE_PROJECT").unwrap());
    let output = std::path::PathBuf::from(std::env::var_os("ANTITONE_BW_PROBE_REPORT").unwrap());
    assert!(!output.exists());
    let project = crate::project::Project::load(&path).unwrap();
    assert!(
        project.scan.as_ref().is_none_or(|s| s.flat_field.is_none()),
        "this diagnostic does not implement captured flat correction"
    );
    let id = project.order[0];
    let source = project.source(id).unwrap();
    let white_balance = project.input.is_negative().then_some([1.; 4]);
    let mut scan = crate::scan::Scan::open(&source.path, white_balance).unwrap();
    scan.reference_exposure = project.exposure_metadata.as_ref().and_then(|m| {
        m.entries
            .iter()
            .find(|e| e.source == id)
            .map(|e| e.reference.clone())
    });
    assert!(
        project.input.is_negative() || scan.reference_exposure.is_some(),
        "Digital must retain its recorded starting exposure"
    );
    let image = crate::scan::ImageSource::new(&scan, project.input).unwrap();
    let roll = crate::processing::RollPipeline::new(project.roll).unwrap();
    let area = image.area();
    let mut inputs = Vec::new();
    for row in 0..128 {
        for column in 0..192 {
            let point = [
                area.x + ((column as f64 + 0.5) / 192. * area.width as f64) as u32,
                area.y + ((row as f64 + 0.5) / 128. * area.height as f64) as u32,
            ];
            inputs.push(roll.apply(image.sample(point).unwrap()).unwrap());
        }
    }
    let min = photographic::decode(super::super::super::authoring::DOMAIN_MIN);
    let max = photographic::decode(super::super::super::authoring::domain_max());
    let outside = inputs
        .iter()
        .filter(|v| v.min_element() < min || v.max_element() > max)
        .count();
    inputs.retain(|v| v.min_element() >= min && v.max_element() <= max);
    assert!(inputs.len() > 1000);
    let mut results = Vec::new();
    pollster::block_on(async {
        let adapter = crate::gpu::instance()
            .request_adapter(&Default::default())
            .await
            .unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let reference = crate::looks::reference::ReferenceTransforms::new(&device, &queue)
            .await
            .unwrap();
        let gate = SubmissionGate::default();
        for paper in Paper::all() {
            for negative in std::iter::once(None).chain(Negative::all().map(Some)) {
                let selection = Selection { negative, paper };
                let model = Model::new(selection);
                let asset = model
                    .bake(
                        crate::looks::authoring::DEFAULT_GRID,
                        &AtomicBool::new(false),
                    )
                    .unwrap();
                let rgba = |encoded: DVec3| {
                    let rgb = encoded.map(photographic::decode);
                    [rgb.x as f32, rgb.y as f32, rgb.z as f32, 1.]
                };
                let expected = inputs
                    .iter()
                    .map(|v| rgba(model.encoded(*v).unwrap()))
                    .collect::<Vec<_>>();
                let actual = inputs
                    .iter()
                    .map(|v| {
                        rgba(
                            asset
                                .lut()
                                .sample_encoded(v.map(photographic::encode))
                                .unwrap(),
                        )
                    })
                    .collect::<Vec<_>>();
                let expected = reference
                    .apply(&device, &queue, &gate, false, &expected)
                    .unwrap();
                let actual = reference
                    .apply(&device, &queue, &gate, false, &actual)
                    .unwrap();
                let mut errors = expected
                    .iter()
                    .zip(&actual)
                    .enumerate()
                    .map(|(i, (a, b))| {
                        (
                            (0..3)
                                .map(|c| (a[c] as f64 - b[c] as f64).abs())
                                .fold(0., f64::max),
                            i,
                        )
                    })
                    .collect::<Vec<_>>();
                errors.sort_by(|a, b| a.0.total_cmp(&b.0));
                let worst = errors.last().unwrap();
                let quantile = |q: f64| errors[((errors.len() - 1) as f64 * q) as usize].0;
                results.push(serde_json::json!({"selection": selection.name(),
                    "p50":quantile(0.5),"p95":quantile(0.95),"p99":quantile(0.99),
                    "maximum":worst.0,"worst_input":inputs[worst.1].to_array()}));
            }
        }
    });
    let report = serde_json::json!({"project":path,"source":source.path,
        "input":project.input,"roll":project.roll,"reference_exposure":scan.reference_exposure,
        "local_grades_and_texture_evaluated":false,"samples":inputs.len(),
        "outside_lut_window_reported_separately":outside,"signed_samples":inputs.iter().filter(|v|v.min_element()<0.).count(),
        "hdr_samples":inputs.iter().filter(|v|v.max_element()>1.).count(),
        "comparison":"Scalar native material versus f64 tetrahedral reference on generated f32 cube; both through fixed GPU authoring view. Photographic input distribution, not a composed-export oracle.","results":results});
    std::fs::write(output, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    println!("{report}");
}
