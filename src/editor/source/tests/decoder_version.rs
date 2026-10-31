use super::*;
use crate::{
    editor::Correction,
    exposure_metadata::{Entry, Metadata},
    exposure_reference::{ReferenceExposure, StartingExposure},
    processing::{Calibration, InputSettings},
    project::{DEVELOPMENT_CONTRACT, DNG_DEVELOPMENT_CONTRACT, FlatFieldSettings, Project, Save},
    raw::sony_fixture,
};
use glam::DVec3;

#[test]
fn recorded_decoder_releases_do_not_block_open_preparation_or_export() {
    let source = sony_fixture::source_with_as_shot(sony_fixture::Order::Little, vec![], [2, 1, 3]);
    let source =
        crate::preview_fixture::append(source.path(), [32, 32], &[128; 32 * 32 * 3], true, Some(2));
    let (device, queue, renderer) = crate::gpu_export::tests::context();
    let gate = crate::gpu::SubmissionGate::default();
    let renderers = crate::renderer::RendererCache::new(gate.clone());
    let directory = tempfile::tempdir().unwrap();
    for (sdk, path) in [
        (false, source.path().to_owned()),
        (true, fixture("preview.dng")),
    ] {
        for digital in [false, true] {
            let mut document = Document::default();
            document.project.input = if digital {
                InputSettings::Digital
            } else {
                Calibration {
                    dmin: DVec3::splat(0.8),
                    dmax: DVec3::splat(0.02),
                    input_floor: crate::processing::CAMERA_INPUT_FLOOR,
                }
                .into()
            };
            document.import(std::slice::from_ref(&path)).unwrap();
            let id = document.project.order[0];
            let record = document.project.source_mut(id).unwrap();
            assert_eq!(
                record.development.decode.contract,
                if sdk {
                    DNG_DEVELOPMENT_CONTRACT
                } else {
                    DEVELOPMENT_CONTRACT
                },
                "the fixture must exercise the named decoder"
            );
            record.development.decode.backend_version = "previous decoder release".into();
            record.adjustment_layers.as_mut().unwrap().layers[0]
                .adjustment
                .photo
                .exposure = 0.375;
            document.project.roll.exposure = 0.25;
            if !digital {
                let area = document.project.source(id).unwrap().development.active_area;
                let map = Arc::new(
                    crate::flat_field::FlatField::from_nodes(
                        area,
                        [2, 2],
                        vec![DVec3::splat(1.1); 4],
                    )
                    .unwrap(),
                );
                document.project.set_flat_field(Some(FlatFieldSettings::new(
                    document.project.source(id).unwrap(),
                    map,
                )));
            }
            let baseline = ReferenceExposure::Matched(StartingExposure {
                ev: 0.5,
                samples: 512,
                spread: 0.1,
            });
            if digital {
                document.exposure_metadata = Arc::new(Metadata {
                    entries: vec![Entry {
                        source: id,
                        reference: baseline.clone(),
                    }],
                    ..Default::default()
                });
            }
            let project_path = directory
                .path()
                .join(format!("legacy-{sdk}-{digital}.antitone"));
            let saved = document.save(&project_path, Save::New).unwrap();
            let legacy = Project::load(&project_path).unwrap();
            let legacy_record = legacy.source(id).unwrap().clone();
            assert_eq!(
                legacy_record.development.decode.backend_version,
                "previous decoder release"
            );
            assert!(
                crate::embedded_preview::read(&path, &legacy_record.development)
                    .unwrap()
                    .is_some()
            );
            let opened = Document::open(&project_path).unwrap();
            assert!(
                !opened.dirty(),
                "decoder provenance refresh must not edit the grade"
            );
            let mut expected = saved.project.clone();
            expected
                .source_mut(id)
                .unwrap()
                .development
                .decode
                .backend_version = if sdk {
                crate::dng::INTERPRETATION.into()
            } else {
                crate::raw::version()
            };
            assert_eq!(opened.project, expected);
            assert_eq!(opened.exposure_metadata, saved.exposure_metadata);
            let opened: DocumentScope = opened.into();
            let correction = Correction::prepare(&device, &opened, None).unwrap();
            assert_eq!(correction.is_some(), !digital);
            let loader = source_loading::SourceLoader::new();
            // Deliberately request the saved descriptor too, not only the refreshed document record.
            let developed = Request {
                reference: opened.exposure_slot(&opened.exposures, id),
                input: opened.project.input,
                path: path.clone(),
                wb: Some(legacy_record.development.decode.fixed_wb),
                expected: Some(legacy_record),
                file: opened.file_key(&path),
                sources: Some(loader.provider(Some(opened.cache.clone()))),
                correction: correction.clone(),
            }
            .load(&loader, &|| false)
            .unwrap();
            let scan = developed.context.scan().unwrap();
            assert_eq!(
                scan.source()
                    .metadata()
                    .development(scan.fixed_wb)
                    .backend_version,
                if sdk {
                    crate::dng::INTERPRETATION.into()
                } else {
                    crate::raw::version()
                }
            );
            if digital {
                assert_eq!(developed.context.reference, Some(baseline));
            }
            let ready = developed
                .prepare_gpu(&renderers, &device, &queue, &|| false)
                .unwrap();
            assert_eq!(ready.area, scan.area);
            let recipe = legacy.image_recipe(id).unwrap();
            assert_eq!(
                recipe.neutral.source_development.decode.backend_version,
                "previous decoder release"
            );
            assert!(
                crate::repair_source::resolve(&scan, &recipe)
                    .unwrap()
                    .is_empty()
            );
            let composition = recipe.composition;
            let output = directory.path().join(format!("legacy-{sdk}-{digital}.tif"));
            crate::gpu_export::job::write(
                &renderer,
                &device,
                (&queue, &gate),
                crate::gpu_export::job::Job {
                    scan: &scan,
                    field: correction.as_ref().map(|field| field.gpu.clone()),
                    recipe: &recipe,
                    format: crate::gpu_export::Format::Tiff(crate::rendering::OutputTarget::Srgb),
                },
                (&output, crate::export::Overwrite::Refuse),
                &mut crate::dust_repair::Execution::default(),
                || Ok(()),
            )
            .unwrap();
            let mut decoder =
                tiff::decoder::Decoder::new(std::fs::File::open(output).unwrap()).unwrap();
            assert_eq!(
                decoder.dimensions().unwrap(),
                (composition.size()[0], composition.size()[1])
            );
            assert!(matches!(
                decoder.read_image().unwrap(),
                tiff::decoder::DecodingResult::U16(_)
            ));
            loader.close_and_wait().unwrap();
        }
    }
}
