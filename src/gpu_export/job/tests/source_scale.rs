use super::*;
use crate::processing::InputSettings;

#[test]
fn digital_source_scale_reaches_neutral_repairs_and_scene_linear_exports_at_zero_ev() {
    let (device, queue) = pollster::block_on(async {
        let adapter = crate::gpu::instance()
            .request_adapter(&Default::default())
            .await
            .unwrap();
        crate::gpu::submission::request_device(&adapter, &Default::default())
            .await
            .unwrap()
    });
    let context = evaluation::Context::new(&device, Default::default());
    let directory = tempfile::tempdir().unwrap();
    for (rgb, expected) in [
        ([2949, 11796, 5898], 0.18_f32),
        ([5898, 23592, 11796], 0.36_f32),
    ] {
        let file = crate::raw::fixture::bayer_dng_with_as_shot(rgb, [4, 1, 2]);
        let scan = std::sync::Arc::new(
            DevelopedSource::from_raw(
                crate::raw::RawFile::open_uncorrected(file.path()).unwrap(),
                None,
            )
            .unwrap(),
        );
        let area = scan.area();
        let mut project = Project {
            input: InputSettings::Digital,
            ..Default::default()
        };
        let id = project
            .add_source(Source {
                capture_time: None,
                development_mode: crate::project::DevelopmentMode::Unity,
                id: SourceId(0),
                path: file.path().into(),
                development: SourceDevelopment {
                    decode: scan.metadata().development(scan.white_balance()),
                    dimensions: [64; 2],
                    active_area: [area.x, area.y, area.width, area.height],
                    camera_model: scan.camera_model().to_owned(),
                    camera_to_ap1: scan.camera_to_ap1().to_cols_array_2d(),
                },
                adjustment_layers: None,
                repairs: Default::default(),
                orientation: Default::default(),
                crop: None,
            })
            .unwrap();
        project
            .source_mut(id)
            .unwrap()
            .repairs
            .insert(
                SupportMask::new([31, 31], [3, 3], vec![true; 9]).unwrap(),
                RepairOrigin::Manual,
                [-16, 0],
            )
            .unwrap();
        assert_eq!(project.roll.exposure, 0.);
        assert!(
            project
                .source(id)
                .unwrap()
                .grade_snapshot()
                .unwrap()
                .layers()
                .iter()
                .all(|layer| layer.photo.exposure == 0.)
        );
        let prepared = crate::photo_preparation::native(
            None,
            &std::sync::OnceLock::new(),
            project.input,
            &scan,
        )
        .unwrap();
        let recipe = project
            .image_recipe_with_reference(project.source(id).unwrap(), prepared.reference)
            .unwrap();
        let healed = crate::repair_source::evaluate(
            &crate::scan::ImageSource::new(&scan, InputSettings::Digital, None, None).unwrap(),
            &recipe.repairs[0],
        )
        .unwrap();
        assert_eq!(healed.pixels().count(), 9);
        for (_, pixel) in healed.pixels() {
            for channel in pixel {
                assert!((channel - expected).abs() < 5e-5, "{pixel:?}");
            }
        }
        let path = directory.path().join(format!("{expected}.exr"));
        write(
            &context,
            &queue,
            Job {
                source: &scan,
                field: None,
                recipe: &recipe,
                render: Render::resolve::<Error>(Format::Exr(ExrPrecision::Full).plan(), || {
                    panic!("scene rendering must not acquire display resources")
                })
                .unwrap(),
            },
            (&path, Overwrite::Refuse),
            &mut Execution::default(),
            || Ok(()),
        )
        .unwrap();
        let image = exr::prelude::read_first_flat_layer_from_file(&path).unwrap();
        assert_eq!(image.layer_data.channel_data.list.len(), 3);
        for channel in &image.layer_data.channel_data.list {
            let exr::prelude::FlatSamples::F32(values) = &channel.sample_data else {
                panic!("full precision EXR expected")
            };
            for point in [[16, 16], [32, 32], [48, 48]] {
                let value = values[point[0] + point[1] * 64];
                assert!((value - expected).abs() < 5e-5, "{value} vs {expected}");
            }
        }
    }
}
