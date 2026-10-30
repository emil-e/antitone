use super::*;
use crate::dust_repair::{RepairOrigin, SupportMask};
use crate::working_precision;

fn exr_pixels(path: &std::path::Path, precision: ExrPrecision) -> Vec<[f64; 3]> {
    use exr::prelude::{FlatSamples, read_first_flat_layer_from_file};
    let image = read_first_flat_layer_from_file(path).unwrap();
    let mut pixels = vec![[0.; 3]; image.layer_data.size.0 * image.layer_data.size.1];
    assert_eq!(image.layer_data.channel_data.list.len(), 3);
    for channel in &image.layer_data.channel_data.list {
        let c = match channel.name.to_string().as_str() {
            "R" => 0,
            "G" => 1,
            "B" => 2,
            _ => panic!("unexpected EXR channel"),
        };
        match (&channel.sample_data, precision) {
            (FlatSamples::F16(values), ExrPrecision::Half) => {
                assert_eq!(values.len(), pixels.len());
                for (pixel, value) in pixels.iter_mut().zip(values) {
                    pixel[c] = value.to_f64();
                }
            }
            (FlatSamples::F32(values), ExrPrecision::Full) => {
                assert_eq!(values.len(), pixels.len());
                for (pixel, value) in pixels.iter_mut().zip(values) {
                    pixel[c] = f64::from(*value);
                }
            }
            _ => panic!("wrong EXR precision"),
        }
    }
    pixels
}

fn edge_border(area: Rect) -> SupportMask {
    // A source-edge block cannot satisfy the biharmonic exterior. It is inside
    // the active area, so the repair instruction validates, but a crop inside
    // the source has no coverage of it.
    SupportMask::new([area.x, area.y], [16, 16], vec![true; 256]).unwrap()
}

#[test]
fn film_mtf_includes_repair_support_just_outside_the_crop() {
    let mut editor = crate::editor::tests::authored_fixture(&["roll-0.dng"]);
    editor.set_dmin(Some(DVec3::splat(0.8)));
    editor.set_dmax(Some(DVec3::splat(0.02)));
    let id = editor.document.visit.current_source.unwrap();
    let area = editor
        .document
        .project
        .source(id)
        .unwrap()
        .development
        .active_area;
    let area = Rect {
        x: area[0],
        y: area[1],
        width: area[2],
        height: area[3],
    };
    editor
        .document
        .project
        .source_mut(id)
        .unwrap()
        .repairs
        .insert(edge_border(area), RepairOrigin::Manual, [30, 0])
        .unwrap();
    editor.document.visit.crop = Some(crate::crop::Crop {
        center: [area.x as f64 + 25., area.y as f64 + 32.],
        size: [32, 32],
        angle: 0.,
    });
    editor.sync_document();
    let clean = editor.document.state.project.image_recipe(id).unwrap();
    let scan = Scan::open(
        &editor.document.source_path(id).unwrap(),
        Some(clean.neutral.source_development.decode.fixed_wb),
    )
    .unwrap();
    assert!(
        crate::repair_source::resolve(&scan, &clean).is_err(),
        "source-wide donor witness"
    );
    assert!(
        crate::repair_source::resolve_output(&scan, &clean)
            .unwrap()
            .is_empty()
    );
    editor.document.film_emulation = FilmEmulationSettings {
        enabled: true,
        amount: 0.,
        size: crate::film_emulation::MAX_SIZE,
        image_sigma: crate::film_emulation::MAX_SIZE,
        ..Default::default()
    };
    editor.sync_document();
    let film = editor.document.state.project.image_recipe(id).unwrap();
    let error = crate::repair_source::resolve_output(&scan, &film).unwrap_err();
    assert!(error.to_string().contains("repair"), "{error}");
}

#[test]
fn authored_reopened_cold_exports_apply_all_repairs_and_failed_preparation_preserves_destinations()
{
    pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let mut editor = Editor::new(&device, &queue);
        editor.document.thumbnails =
            (Thumbnails::default()).with_sources(editor.document.cache.clone());
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
        editor.sync_document();
        let nodes = (0..8)
            .map(|i| {
                [
                    -1.98 + 4. * (i % 2) as f32,
                    -1.98 + 4. * ((i / 2) % 2) as f32,
                    -1.98 + 4. * (i / 4) as f32,
                ]
            })
            .collect();
        let look = Arc::new(
            crate::looks::LookAsset::new(
                "Captured export look".into(),
                crate::looks::LookOrigin::Imported,
                Arc::new(crate::looks::Lut3D::new(2, [-2.; 3], [2.; 3], nodes).unwrap()),
            )
            .unwrap(),
        );
        editor.document.state.project.output.look = Some(look.clone());
        let id = editor.document.visit.current_source.unwrap();
        editor
            .document
            .project
            .source_mut(id)
            .unwrap()
            .repairs
            .insert(
                SupportMask::new([126, 62], [5, 5], vec![true; 25]).unwrap(),
                RepairOrigin::Manual,
                [-64, 0],
            )
            .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let saved = directory.path().join("repaired.antitone");
        editor
            .document
            .project
            .save(&saved, crate::project::Save::New)
            .unwrap();
        editor = crate::editor::tests::complete_document_operation(
            editor,
            Duration::from_secs(30),
            |editor, cx| editor.start_document(Preparation::New, cx),
        );
        editor = crate::editor::tests::complete_document_operation(
            editor,
            Duration::from_secs(30),
            |editor, cx| editor.start_document(Preparation::Open(saved), cx),
        );
        let id = editor.document.visit.current_source.unwrap();
        let dirty = editor.document.dirty();
        let recipe = editor.document.state.project.image_recipe(id).unwrap();
        let scan = Scan::open(
            &editor.document.source_path(id).unwrap(),
            Some(recipe.neutral.source_development.decode.fixed_wb),
        )
        .unwrap();
        let pipeline = crate::adjustment_layers::AdjustmentLayerPipeline::new(
            recipe.neutral.processing_input().unwrap(),
            recipe.roll,
            recipe.grade.clone(),
            scan.camera_to_ap1,
        )
        .unwrap();
        let repairs = crate::repair_source::resolve_output(&scan, &recipe).unwrap();
        assert_eq!(repairs.len(), 1);
        let expected_exr: Vec<_> = repairs[0]
            .pixels()
            .map(|(point, neutral)| {
                let graded = pipeline
                    .grade_neutral(
                        crate::processing::InvertedPixel {
                            rgb: DVec3::from_array(neutral.map(f64::from)),
                            floored: [false; 3],
                        },
                        point,
                    )
                    .unwrap();
                let frame = recipe
                    .composition
                    .source_to_frame(crate::geometry::SourcePoint(
                        point.map(|v| f64::from(v) + 0.5),
                    ));
                let index = frame.0[1].floor() as usize * recipe.composition.size()[0] as usize
                    + frame.0[0].floor() as usize;
                (index, crate::color::ap1_to_ap0() * graded.rgb)
            })
            .collect();
        assert_eq!(expected_exr.len(), 25);
        for (name, format) in [
            ("repaired.tif", OutputFormat::Tiff(OutputTarget::Srgb)),
            ("repaired-half.exr", OutputFormat::Exr(ExrPrecision::Half)),
            ("repaired-full.exr", OutputFormat::Exr(ExrPrecision::Full)),
        ] {
            assert!(
                !editor
                    .document
                    .dust
                    .has_evaluation(id, crate::dust_repair::RepairId(1)),
                "export must not require or warm the viewer cache"
            );
            let path = directory.path().join(name);
            editor = wait(editor, |editor, cx| {
                editor.export(path.clone(), format, Overwrite::Refuse, cx);
                editor.document.state.project.output.look = None;
            });
            editor.document.state.project.output.look = Some(look.clone());
            assert!(editor.failure.is_none(), "{}", editor.status);
            assert!(path.exists());
            match format {
                OutputFormat::Tiff(_) => {
                    assert!(!tiff_pixels(&path).is_empty());
                }
                OutputFormat::Exr(precision) => {
                    let pixels = exr_pixels(&path, precision);
                    let size = recipe.composition.size();
                    assert_eq!(pixels.len(), size[0] as usize * size[1] as usize);
                    for (index, expected) in &expected_exr {
                        for c in 0..3 {
                            let expected = expected[c];
                            // Shared FP32 working comparison, plus file-storage rounding.
                            let quantization = match precision {
                                ExrPrecision::Half => {
                                    0.0009765625 * expected.abs() + 5.960464477539063e-8
                                }
                                ExrPrecision::Full => f64::from(f32::EPSILON) * expected.abs(),
                            };
                            assert!(
                                (pixels[*index][c] - expected).abs()
                                    <= working_precision::allowance(expected) + quantization,
                                "{precision:?} repair pixel {index}, channel {c}: {} vs {expected}",
                                pixels[*index][c]
                            );
                        }
                    }
                }
            }
        }
        assert_eq!(editor.document.dirty(), dirty);
        let area = editor.document.visit.canvas.ready.as_ref().unwrap().area;
        let border = editor
            .document
            .project
            .source_mut(id)
            .unwrap()
            .repairs
            .insert(edge_border(area), RepairOrigin::Manual, [64, 0])
            .unwrap();
        editor.document.visit.crop = Some(crate::crop::Crop {
            center: [48., 48.],
            size: [32, 32],
            angle: 0.2,
        });
        editor.sync_document();
        let recipe = editor.document.state.project.image_recipe(id).unwrap();
        let scan = Scan::open(
            &editor.document.source_path(id).unwrap(),
            Some(recipe.neutral.source_development.decode.fixed_wb),
        )
        .unwrap();
        assert!(
            crate::repair_source::resolve(&scan, &recipe).is_err(),
            "the source-edge border cannot satisfy the biharmonic exterior"
        );
        assert!(
            crate::repair_source::resolve_output(&scan, &recipe)
                .unwrap()
                .is_empty(),
            "zero exact output coverage, including a hole inside a support rectangle, excludes irrelevant repairs"
        );
        let cropped = directory.path().join("outside-failed-repair.tif");
        editor = wait(editor, |editor, cx| {
            editor.export(
                cropped.clone(),
                OutputFormat::Tiff(OutputTarget::Srgb),
                Overwrite::Refuse,
                cx,
            )
        });
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
        editor.document.visit.crop = None;
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
        editor = wait(editor, |editor, cx| {
            editor.export(
                plain.clone(),
                OutputFormat::Tiff(OutputTarget::Srgb),
                Overwrite::Refuse,
                cx,
            )
        });
        assert!(editor.failure.is_none(), "{}", editor.status);
        assert_ne!(
            tiff_pixels(&plain),
            tiff_pixels(&directory.path().join("repaired.tif")),
            "cold export must actually apply the captured repair"
        );
        for (name, precision) in [("half", ExrPrecision::Half), ("full", ExrPrecision::Full)] {
            let plain = directory.path().join(format!("plain-{name}.exr"));
            editor = wait(editor, |editor, cx| {
                editor.export(
                    plain.clone(),
                    OutputFormat::Exr(precision),
                    Overwrite::Refuse,
                    cx,
                )
            });
            assert!(editor.failure.is_none(), "{}", editor.status);
            let repaired = exr_pixels(
                &directory.path().join(format!("repaired-{name}.exr")),
                precision,
            );
            let plain = exr_pixels(&plain, precision);
            assert!(
                expected_exr
                    .iter()
                    .any(|(index, _)| repaired[*index] != plain[*index]),
                "{precision:?} fixture must distinguish repaired from unrepaired EXR"
            );
        }
        let before = directory.path().join("repaired.tif");
        let bytes = std::fs::read(&before).unwrap();
        let area = editor.document.visit.canvas.ready.as_ref().unwrap().area;
        let repairs = &mut editor
            .document
            .state
            .project
            .source_mut(id)
            .unwrap()
            .repairs;
        repairs.clear();
        repairs
            .insert(edge_border(area), RepairOrigin::Manual, [64, 0])
            .unwrap();
        editor = wait(editor, |editor, cx| {
            editor.export(
                before.clone(),
                OutputFormat::Tiff(OutputTarget::Srgb),
                Overwrite::Confirmed,
                cx,
            )
        });
        assert!(editor.failure.is_some());
        assert!(
            editor.status.contains("repair") && editor.status.contains("two-pixel exterior"),
            "{}",
            editor.status
        );
        assert_eq!(std::fs::read(before).unwrap(), bytes);
    });
}
