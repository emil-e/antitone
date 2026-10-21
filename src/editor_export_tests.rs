use super::*;
use tiff::decoder::{Decoder, DecodingResult};

#[path = "editor_export_production_tests.rs"]
mod production;
#[path = "editor_export_repair_tests.rs"]
mod repairs;

fn wait(editor: Editor) -> Editor {
    super::tests::complete_export_operation(editor, Duration::from_secs(30))
}

#[test]
fn export_completion_cannot_retire_new_source_or_document_operations() {
    for document in [false, true] {
        let mut editor = super::tests::fixture();
        super::tests::add_ready_fixture(&mut editor);
        editor.sync_document();
        editor.export(
            "captured.exr".into(),
            OutputFormat::Exr(ExrPrecision::Full),
            Overwrite::Refuse,
        );
        assert!(editor.export_operation.request.is_some());
        let export = editor.export_operation.pending.unwrap();
        if document {
            editor.start_document(Preparation::New);
        } else {
            editor.load_source("another.ARW".into(), false);
        }
        assert!(if document {
            editor.candidate_operation.request.is_some()
        } else {
            editor.document.visit.source.request.is_some()
        });
        let status = editor.status.clone();
        let title = editor.tasks.view().unwrap().title;
        let project = editor.document.project.clone();
        editor.export_result(export, Ok("captured.exr".into()));
        assert!(editor.busy);
        assert!(editor.export_operation.pending.is_none());
        assert_eq!(editor.status, status);
        assert_eq!(editor.tasks.view().unwrap().title, title);
        assert_eq!(editor.document.project, project);
        if document {
            editor.document_prepared(
                editor.document_pending.unwrap(),
                Err("new document failed".into()),
            );
        } else {
            editor.source_result(editor.id, Err("new source failed".into()));
        }
        assert!(!editor.busy);
        assert!(editor.tasks.view().is_none());
    }
}

fn tiff_pixels(path: &std::path::Path) -> Vec<u16> {
    let mut decoder = Decoder::new(std::fs::File::open(path).unwrap()).unwrap();
    let DecodingResult::U16(values) = decoder.read_image().unwrap() else {
        panic!()
    };
    values
}

#[test]
#[ignore = "supplied read-only M9; set ANTITONE_M9_SAMPLE, full decode with bounded exports"]
fn supplied_m9_editor_import_save_open_and_bounded_exports() {
    let path =
        PathBuf::from(std::env::var_os("ANTITONE_M9_SAMPLE").expect("set ANTITONE_M9_SAMPLE"));
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let mut editor = Editor::new(&device, &queue, None);
        editor.thumbnails = Thumbnails::default();
        editor.add_raws(vec![path]);
        editor = crate::editor::tests::complete_document_operation(editor, Duration::from_secs(30));
        assert!(editor.document.visit.ready.is_some(), "{}", editor.status);
        assert_eq!(
            editor
                .document
                .project
                .development
                .as_ref()
                .unwrap()
                .contract,
            crate::project::DNG_DEVELOPMENT_CONTRACT
        );
        assert!(
            editor.document.project.sources[0]
                .development
                .dng_development
                .is_some()
        );
        editor.dmin = Some(DVec3::splat(0.75));
        editor.dmax = Some(DVec3::splat(0.01));
        editor.roll = RollSettings::new_grade();
        // Exercise the real full-resolution source while keeping generated files tiny.
        // These declared anchors are plumbing inputs, not a grade of this camera photograph.
        editor.crop = Some(crate::crop::Crop {
            center: [1024., 1024.],
            size: [32, 32],
            angle: 0.,
        });
        editor.sync_document();
        let directory = tempfile::tempdir().unwrap();
        let project_path = directory.path().join("m9.antitone");
        let saved = editor
            .document
            .save(&project_path, crate::project::Save::New)
            .unwrap();
        editor.start_document(Preparation::Open(project_path));
        editor = crate::editor::tests::complete_document_operation(editor, Duration::from_secs(30));
        assert!(
            editor.document.visit.ready.is_some() && editor.failure.is_none(),
            "{}",
            editor.status
        );
        assert_eq!(editor.document.project, saved.project);
        for (name, format) in [
            ("m9.tif", OutputFormat::Tiff(OutputTarget::Srgb)),
            ("m9.exr", OutputFormat::Exr(ExrPrecision::Full)),
        ] {
            let output = directory.path().join(name);
            editor.export(output.clone(), format, Overwrite::Refuse);
            editor = wait(editor);
            assert!(editor.failure.is_none(), "{}", editor.status);
            match format {
                OutputFormat::Tiff(_) => {
                    let mut decoder = Decoder::new(std::fs::File::open(output).unwrap()).unwrap();
                    assert_eq!(decoder.dimensions().unwrap(), (32, 32));
                    let DecodingResult::U16(values) = decoder.read_image().unwrap() else {
                        panic!("expected RGB16")
                    };
                    assert_eq!(values.len(), 32 * 32 * 3);
                }
                OutputFormat::Exr(_) => {
                    let image = exr::prelude::read_first_flat_layer_from_file(output).unwrap();
                    assert_eq!((image.layer_data.size.0, image.layer_data.size.1), (32, 32));
                    assert_eq!(image.layer_data.channel_data.list.len(), 3);
                    assert!(image.attributes.chromaticities.is_some());
                }
            }
        }
    });
}

#[test]
fn authored_gpu_export_worker_preserves_snapshots_files_and_operation_identity_without_ocio() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let mut editor = Editor::new(&device, &queue, None);
        editor.thumbnails = Thumbnails::default();
        editor.add_raws(vec![
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        ]);
        editor = crate::editor::tests::complete_document_operation(editor, Duration::from_secs(30));
        assert!(editor.document.visit.ready.is_some(), "{}", editor.status);
        editor.dmin = Some(DVec3::splat(3000. / 4031.));
        editor.dmax = Some(DVec3::splat(128. / 4031.));
        editor.roll = RollSettings::new_grade();
        editor.photo = PhotoSettings {
            contrast: 0.8,
            saturation: 1.3,
            ..Default::default()
        };
        editor.sync_document();
        let before = editor.document.project.clone();
        let directory = tempfile::tempdir().unwrap();
        for (name, format) in [
            ("srgb.tif", OutputFormat::Tiff(OutputTarget::Srgb)),
            ("p3.tif", OutputFormat::Tiff(OutputTarget::DisplayP3)),
            ("half.exr", OutputFormat::Exr(ExrPrecision::Half)),
            ("full.exr", OutputFormat::Exr(ExrPrecision::Full)),
        ] {
            let path = directory.path().join(name);
            editor.export(path.clone(), format, Overwrite::Refuse);
            assert!(editor.busy && editor.export_operation.pending.is_some());
            editor = wait(editor);
            assert!(editor.failure.is_none(), "{}", editor.status);
            assert!(editor.export_operation.pending.is_none() && editor.tasks.view().is_none());
            match format {
                OutputFormat::Tiff(target) => {
                    let mut decoder = Decoder::new(std::fs::File::open(&path).unwrap()).unwrap();
                    let area = editor.document.visit.ready.as_ref().unwrap().area;
                    let size = [area.width, area.height];
                    assert_eq!(decoder.dimensions().unwrap(), (size[0], size[1]));
                    assert_eq!(
                        decoder
                            .get_tag_u8_vec(tiff::tags::Tag::Unknown(34675))
                            .unwrap(),
                        crate::profiles::icc(target)
                    );
                    assert!(!tiff_pixels(&path).is_empty());
                }
                OutputFormat::Exr(_) => {
                    let image = exr::prelude::read_first_flat_layer_from_file(&path).unwrap();
                    assert_eq!(image.layer_data.channel_data.list.len(), 3);
                    assert!(image.attributes.chromaticities.is_some());
                }
            }
            assert_eq!(editor.document.project, before);
        }
        let source = directory.path().join("srgb.tif");
        let expected = tiff_pixels(&source);
        let captured = directory.path().join("captured.tif");
        editor.export(
            captured.clone(),
            OutputFormat::Tiff(OutputTarget::Srgb),
            Overwrite::Refuse,
        );
        assert!(editor.export_operation.request.is_some());
        // Later accepted edits/source identity cannot change the captured file recipe.
        editor.photo.exposure = 0.75;
        editor.sync_document();
        editor.id += 7;
        let edited = editor.document.project.clone();
        editor = wait(editor);
        assert!(editor.failure.is_none(), "{}", editor.status);
        assert_eq!(tiff_pixels(&captured), expected);
        assert_eq!(editor.document.project, edited);
        assert_eq!(editor.photo.exposure, 0.75);
        // A refused destination remains intact even after the visible authority and grade change.
        let bytes = std::fs::read(&source).unwrap();
        editor.export(
            source.clone(),
            OutputFormat::Tiff(OutputTarget::Srgb),
            Overwrite::Refuse,
        );
        editor = wait(editor);
        assert!(editor.failure.is_some());
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
        assert_eq!(editor.document.project, edited);
    });
}
