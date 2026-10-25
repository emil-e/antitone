use super::*;
use crate::dust_repair::{DonorChoice, RepairOrigin, SupportMask};

#[test]
fn authored_production_worker_keeps_captured_repairs_look_and_destination_safe_on_cancellation() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let mut editor = Editor::new(&device, &queue);
        editor.thumbnails = Thumbnails::default();
        editor = crate::editor::tests::complete_document_operation(
            editor,
            Duration::from_secs(30),
            |editor, cx| {
                editor.add_raws(
                    vec![
                        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                            .join("tests/fixtures/raw/patches.dng"),
                    ],
                    cx,
                )
            },
        );
        editor.set_dmin(Some(DVec3::splat(0.8)));
        editor.set_dmax(Some(DVec3::splat(0.02)));
        editor.roll = RollSettings::default();
        let area = editor.document.visit.ready.as_ref().unwrap().area;
        editor.crop = Some(Crop {
            center: [
                f64::from(area.x) + f64::from(area.width) / 2.,
                f64::from(area.y) + f64::from(area.height) / 2.,
            ],
            size: [area.width - 96, area.height - 96],
            angle: 0.13,
        });
        editor.sync_document();
        let id = editor.current_source.unwrap();
        editor
            .document
            .project
            .source_mut(id)
            .unwrap()
            .repairs
            .insert(
                SupportMask::new(
                    [area.x + area.width / 2, area.y + area.height / 2],
                    [3, 3],
                    vec![true; 9],
                )
                .unwrap(),
                RepairOrigin::Manual,
                DonorChoice::Offset([-32, 0]),
            )
            .unwrap();
        let look = Arc::new(
            crate::looks::LookAsset::new(
                "Worker capture".into(),
                crate::looks::LookOrigin::Imported,
                Arc::new(
                    crate::looks::Lut3D::new(
                        2,
                        [-2.; 3],
                        [2.; 3],
                        (0..8)
                            .map(|i| {
                                [
                                    -1.98 + 4. * (i % 2) as f32,
                                    -1.98 + 4. * ((i / 2) % 2) as f32,
                                    -1.98 + 4. * (i / 4) as f32,
                                ]
                            })
                            .collect(),
                    )
                    .unwrap(),
                ),
            )
            .unwrap(),
        );
        editor.document.project.output.look = Some(look.clone());
        let directory = tempfile::tempdir().unwrap();
        let mut reference: Option<Vec<u16>> = None;
        for with_look in [true, false] {
            editor.document.project.output.look = with_look.then(|| look.clone());
            let path = directory.path().join(format!("look-{with_look}.tif"));
            assert!(
                !editor
                    .document
                    .dust
                    .has_evaluation(id, crate::dust_repair::RepairId(1))
            );
            editor = wait(editor, |editor, cx| {
                editor.export(
                    path.clone(),
                    OutputFormat::Tiff(OutputTarget::DisplayP3),
                    Overwrite::Refuse,
                    cx,
                );
                // Change live state after capture; neither file may follow this newer value.
                editor.document.project.output.look = (!with_look).then(|| look.clone());
            });
            assert!(editor.failure.is_none(), "{}", editor.status);
            editor.document.project.output.look = Some(look.clone());
            let pixels = tiff_pixels(&path);
            let (triplets, tail) = pixels.as_chunks::<3>();
            assert!(tail.is_empty());
            assert!(
                triplets.iter().skip(1).any(|pixel| pixel != &triplets[0]),
                "worker fixture must retain spatial variation"
            );
            if let Some(reference) = &reference {
                assert_ne!(
                    &pixels, reference,
                    "captured look must affect pixels, not metadata alone"
                );
            } else {
                reference = Some(pixels);
            }
            assert!(
                !editor
                    .document
                    .dust
                    .has_evaluation(id, crate::dust_repair::RepairId(1))
            );
        }
        let path = directory.path().join("cancelled.tif");
        std::fs::write(&path, b"existing destination").unwrap();
        let gate = host::App::submission_gate(&editor).clone();
        editor = wait(editor, |editor, cx| {
            gate.configure(|| {
                editor.export(
                    path.clone(),
                    OutputFormat::Tiff(OutputTarget::DisplayP3),
                    Overwrite::Confirmed,
                    cx,
                );
                editor.export_operation.cancelled.as_ref().unwrap().cancel();
            });
        });
        assert!(editor.status.contains("cancel"), "{}", editor.status);
        assert_eq!(std::fs::read(&path).unwrap(), b"existing destination");
        assert_eq!(
            std::fs::read_dir(directory.path()).unwrap().count(),
            3,
            "cancelled job leaves no temporary file"
        );
        assert!(editor.export_operation.cancelled.is_none());
    });
}
