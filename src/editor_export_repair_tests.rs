use super::*;
use crate::dust_repair::{DonorChoice, RepairOrigin, SupportMask};

fn donorless_border(area: Rect) -> SupportMask {
    // The inset admits the biharmonic exterior, while donor padding still spans the source.
    let bits = (0..area.width * area.height)
        .map(|index| {
            let x = index % area.width;
            let y = index / area.width;
            x >= 8
                && y >= 8
                && x + 8 < area.width
                && y + 8 < area.height
                && (x == 8 || y == 8 || x + 9 == area.width || y + 9 == area.height)
        })
        .collect();
    SupportMask::new([area.x, area.y], [area.width, area.height], bits).unwrap()
}

#[test]
fn authored_reopened_cold_exports_apply_all_repairs_and_failed_preparation_preserves_destinations()
{
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
        wait(&mut editor);
        editor.dmin = Some(DVec3::splat(0.8));
        editor.dmax = Some(DVec3::splat(0.02));
        editor.sync_document();
        let id = editor.current_source.unwrap();
        editor
            .document
            .project
            .source_mut(id)
            .unwrap()
            .repairs
            .insert(
                SupportMask::new([126, 62], [5, 5], vec![true; 25]).unwrap(),
                RepairOrigin::Manual,
                DonorChoice::Offset([-64, 0]),
            )
            .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let saved = directory.path().join("repaired.antitone");
        editor
            .document
            .project
            .save(&saved, crate::project::Save::New)
            .unwrap();
        editor.start_document(DocumentWork::New);
        wait(&mut editor);
        editor.start_document(DocumentWork::Open(saved));
        wait(&mut editor);
        let id = editor.current_source.unwrap();
        let dirty = editor.document.dirty();
        for (name, format) in [
            ("repaired.tif", OutputFormat::Tiff(OutputTarget::Srgb)),
            ("repaired-half.exr", OutputFormat::Exr(ExrPrecision::Half)),
            ("repaired-full.exr", OutputFormat::Exr(ExrPrecision::Full)),
        ] {
            assert!(
                !editor
                    .dust
                    .has_evaluation(id, crate::dust_repair::RepairId(1)),
                "export must not require or warm the viewer cache"
            );
            let path = directory.path().join(name);
            editor.export(path.clone(), format, Overwrite::Refuse);
            wait(&mut editor);
            assert!(editor.failure.is_none(), "{}", editor.status);
            assert!(path.exists());
            match format {
                OutputFormat::Tiff(_) => assert!(!tiff_pixels(&path).is_empty()),
                OutputFormat::Exr(_) => {
                    exr::prelude::read_first_flat_layer_from_file(&path).unwrap();
                }
            }
        }
        assert_eq!(editor.document.dirty(), dirty);
        let area = editor.ready.as_ref().unwrap().area;
        let border = editor
            .document
            .project
            .source_mut(id)
            .unwrap()
            .repairs
            .insert(
                donorless_border(area),
                RepairOrigin::Manual,
                DonorChoice::Automatic,
            )
            .unwrap();
        editor.crop = Some(crate::crop::Crop {
            center: [48., 48.],
            size: [32, 32],
            angle: 0.2,
        });
        editor.sync_document();
        let recipe = editor.document.project.image_recipe(id).unwrap();
        let scan = Scan::open(
            &editor.document.source_path(id).unwrap(),
            Some(recipe.neutral.development.fixed_wb),
        )
        .unwrap();
        assert!(
            crate::repair_source::resolve(&scan, &recipe).is_err(),
            "the source-wide border has no complete donor"
        );
        assert!(
            crate::repair_source::resolve_output(&scan, &recipe)
                .unwrap()
                .is_empty(),
            "zero exact output coverage, including a hole inside a support rectangle, excludes irrelevant repairs"
        );
        let cropped = directory.path().join("outside-failed-repair.tif");
        editor.export(
            cropped.clone(),
            OutputFormat::Tiff(OutputTarget::Srgb),
            Overwrite::Refuse,
        );
        wait(&mut editor);
        assert!(editor.failure.is_none(), "{}", editor.status);
        assert_eq!(tiff_pixels(&cropped).len(), 32 * 32 * 3);
        editor
            .document
            .project
            .source_mut(id)
            .unwrap()
            .repairs
            .remove(border)
            .unwrap();
        editor.crop = None;
        editor.sync_document();
        editor
            .document
            .project
            .source_mut(id)
            .unwrap()
            .repairs
            .set_enabled(crate::dust_repair::RepairId(1), false)
            .unwrap();
        let plain = directory.path().join("plain.tif");
        editor.export(
            plain.clone(),
            OutputFormat::Tiff(OutputTarget::Srgb),
            Overwrite::Refuse,
        );
        wait(&mut editor);
        assert!(editor.failure.is_none(), "{}", editor.status);
        assert_ne!(
            tiff_pixels(&plain),
            tiff_pixels(&directory.path().join("repaired.tif")),
            "cold export must actually apply the captured repair"
        );
        let before = directory.path().join("repaired.tif");
        let bytes = std::fs::read(&before).unwrap();
        let area = editor.ready.as_ref().unwrap().area;
        let repairs = &mut editor.document.project.source_mut(id).unwrap().repairs;
        repairs.clear();
        repairs
            .insert(
                donorless_border(area),
                RepairOrigin::Manual,
                DonorChoice::Automatic,
            )
            .unwrap();
        editor.export(
            before.clone(),
            OutputFormat::Tiff(OutputTarget::Srgb),
            Overwrite::Confirmed,
        );
        wait(&mut editor);
        assert!(editor.failure.is_some());
        assert!(
            editor.status.contains("repair") && editor.status.contains("donor"),
            "{}",
            editor.status
        );
        assert_eq!(std::fs::read(before).unwrap(), bytes);
    });
}
