//! Ordinary source acquisition and composed GPU consumers across RAW representations.
use super::*;
use crate::frame_raster::FrameRaster;
use crate::geometry::Composition;
use crate::geometry::FramePoint;
use crate::gpu::submission::SubmissionGate;
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::gpu::view::RenderedView;
use crate::gpu::view::ViewKernels;
use crate::gpu::view::request::SourceRequest;
use crate::processing::Calibration;
use crate::processing::InputSettings;
use crate::rendering::OutputTarget;
use glam::DVec3;

#[path = "../../../../tests/support/sony_ycc_fixture.rs"]
mod sony_ycc;

#[test]
fn authored_prebalanced_rgb_import_reopen_view_and_bounded_exports() {
    let fixture = sony_ycc::source();
    import_reopen_view_and_bounded_exports(&[fixture.path().into()]);
}

#[test]
fn authored_capture_balance_is_per_source_across_input_stage_switches() {
    let first = sony_ycc::source();
    let second = sony_ycc::source_with_balance(Some([4096, 1024, 2048]));
    let mut document = Document::default();
    document
        .import(&[first.path().into(), second.path().into()])
        .unwrap();
    assert!(document.project.input.is_negative());
    let loader = source_loading::SourceLoader::new();
    for input in [InputSettings::Digital, InputSettings::default()] {
        document.redevelop_for_input(input).unwrap();
        assert_eq!(document.project.input, input);
        assert_eq!(document.project.sources.len(), 2);
        for (source, balance) in document
            .project
            .sources
            .iter()
            .zip([[2048., 1024., 1536.], [4096., 1024., 2048.]])
        {
            let scan = pollster::block_on(document.acquire_source(
                source.id,
                &loader,
                &Default::default(),
                &Default::default(),
            ))
            .unwrap()
            .unwrap();
            let expected = if input.is_negative() {
                crate::project::UNITY_WB
            } else {
                [balance[0], balance[1], balance[2], balance[1]]
            };
            assert_eq!(scan.white_balance, expected);
            assert_eq!(
                document
                    .source_key(source.id)
                    .unwrap()
                    .development()
                    .decode
                    .white_balance,
                expected
            );
        }
        document.project.require_runtime().unwrap();
        if input.is_negative() {
            document.project.require_scan_sources().unwrap();
        }
    }
    loader.close_and_wait().unwrap();
}

#[test]
#[ignore = "requires ANTITONE_NON_CFA_SAMPLES (path-separated RAW files) and a GPU"]
fn non_cfa_import_reopen_view_and_bounded_exports() {
    let paths: Vec<_> = std::env::split_paths(
        &std::env::var_os("ANTITONE_NON_CFA_SAMPLES").expect("set ANTITONE_NON_CFA_SAMPLES"),
    )
    .collect();
    assert!(!paths.is_empty());
    for path in &paths {
        let header = crate::raw::RawFile::open(path).unwrap().metadata;
        assert_eq!(
            header.filters,
            0,
            "{} must actually be non-CFA",
            path.display()
        );
    }
    import_reopen_view_and_bounded_exports(&paths);
}

pub(super) fn import_reopen_view_and_bounded_exports(paths: &[PathBuf]) {
    let (device, queue, renderer) = crate::gpu_export::tests::context();
    let gate = SubmissionGate::default();
    let renderers = crate::renderer::RendererCache::new(gate.clone());
    let kernels = pollster::block_on(ViewKernels::new(&device)).unwrap();
    let directory =
        tempfile::tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join(".cache")).unwrap();
    let retained_output = std::env::var_os("ANTITONE_RAW_FORMAT_OUTPUT").map(PathBuf::from);
    let view_size = if retained_output.is_some() {
        [768, 512]
    } else {
        [192, 128]
    };
    let output_root = retained_output
        .clone()
        .unwrap_or_else(|| directory.path().into());
    std::fs::create_dir_all(&output_root).unwrap();
    for (index, path) in paths.iter().enumerate() {
        let before = std::fs::metadata(path).unwrap();
        let header = crate::raw::RawFile::open(path).unwrap().metadata;
        let mut bases = Vec::new();
        for digital in [false, true] {
            let mode = if digital { "digital" } else { "negative" };
            let mut document = Document::default();
            document.project.input = if digital {
                InputSettings::Digital
            } else {
                // Exercise inversion, not calibration quality of these scene photographs.
                Calibration {
                    dmin: DVec3::splat(0.8),
                    dmax: DVec3::splat(0.02),
                }
                .into()
            };
            document.import(std::slice::from_ref(path)).unwrap();
            let document: DocumentScope = document.into();
            let id = document.project.order[0];
            let expected = document.project.source(id).unwrap();
            assert_eq!(expected.orientation, header.photo_orientation().unwrap());
            let loader = source_loading::SourceLoader::new();
            let load = |document: &DocumentScope, loader: &source_loading::SourceLoader| {
                Request {
                    reference: document.exposure_slot(&document.exposures, id),
                    input: document.project.input,
                    path: path.clone(),
                    wb: document.project.acquisition_wb(),
                    expected: Some(document.project.source(id).unwrap().clone()),
                    file: document.file_key(path),
                    sources: Some(loader.provider(Some(document.cache.clone()))),
                    correction: None,
                }
                .load(loader, &|| false)
                .unwrap()
            };
            let developed = load(&document, &loader);
            let scan = developed.context.scan().unwrap();
            assert_eq!(scan.area, header.image_area().unwrap());
            let [width, height] = header.source_dimensions().unwrap();
            assert_eq!(scan.dimensions().unwrap(), (width, height));
            assert_eq!(
                scan.source().white_balance(),
                if digital {
                    header.as_shot_wb()
                } else {
                    crate::project::UNITY_WB
                }
            );
            assert!(scan.camera_to_ap1.is_finite());
            if digital {
                assert!(developed.context.reference.is_some());
                eprintln!(
                    "{} brightness={:?}",
                    path.display(),
                    developed.context.reference
                );
            }
            let (_, _, pixels) = scan.preview_source(128).unwrap();
            assert!(
                pixels
                    .iter()
                    .all(|p| p[..3].iter().all(|v| v.is_finite() && *v >= 0.))
            );
            assert!(pixels.windows(2).any(|p| p[0][..3] != p[1][..3]));
            bases.push(pixels);
            let ready = developed
                .prepare_gpu(&renderers, &device, &queue, &|| false)
                .unwrap();
            assert!(ready.proxy.size().into_iter().all(|n| n > 0 && n <= 1024));
            let composition =
                Composition::new(scan.area, Default::default(), expected.orientation).unwrap();
            let frame = FrameRaster::new(
                composition,
                FramePoint([0., 0.]),
                FramePoint(composition.size().map(f64::from)),
                view_size,
            )
            .unwrap();
            let settings = document.project.input;
            let matrix = scan.positive_matrix(&settings).unwrap();
            let grade = expected.grade_snapshot().unwrap();
            let spec = SourceRequest::new(frame, settings, matrix, None).unwrap();
            let source = ready
                .proxy
                .view_source(
                    &queue,
                    &gate,
                    kernels.clone(),
                    &spec.processing_fixture(),
                    spec,
                )
                .unwrap();
            let input: AuthoredPhotograph = Arc::new(
                AuthoredPhotograph::new(
                    &device.limits(),
                    settings,
                    document.project.roll,
                    grade.clone(),
                    matrix,
                    scan.area,
                )
                .unwrap(),
            )
            .into();
            for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
                let view =
                    RenderedView::new(&device, source.clone(), &input, target, None).unwrap();
                view.ensure_submitted(&device, &queue, &gate, &egui::Context::default());
                let pixels = crate::viewer::readback::image_pixels(&device, &queue, &view.image);
                assert_eq!(pixels.len(), (view_size[0] * view_size[1]) as usize);
                assert!(pixels.iter().flatten().all(|v| v.is_finite()));
                assert!(pixels.windows(2).any(|p| p[0][..3] != p[1][..3]));
                if retained_output.is_some() && target == OutputTarget::Srgb {
                    let rgb: Vec<u8> = pixels
                        .iter()
                        .flat_map(|pixel| {
                            pixel[..3].iter().map(|&value| {
                                (target.encode(f64::from(value).clamp(0., 1.)).unwrap() * 255.)
                                    .round() as u8
                            })
                        })
                        .collect();
                    let output =
                        std::fs::File::create(output_root.join(format!("{index}-{mode}-view.jpg")))
                            .unwrap();
                    image::codecs::jpeg::JpegEncoder::new_with_quality(output, 95)
                        .encode(
                            &rgb,
                            view_size[0],
                            view_size[1],
                            image::ExtendedColorType::Rgb8,
                        )
                        .unwrap();
                }
            }
            let project_path = output_root.join(format!("{index}-{mode}.antitone"));
            let mut captured = document.capture();
            captured
                .project
                .source_mut(id)
                .unwrap()
                .development
                .decode
                .backend_version = "0.22.0-Release".into();
            captured
                .save(&project_path, crate::project::Save::New)
                .unwrap();
            assert_eq!(
                crate::project::Project::load(&project_path)
                    .unwrap()
                    .source(id)
                    .unwrap()
                    .development
                    .decode
                    .backend_version,
                "0.22.0-Release"
            );
            let reopened: DocumentScope = Document::open(&project_path).unwrap().into();
            assert_eq!(
                reopened.project.source(id).unwrap().development,
                captured.project.source(id).unwrap().development
            );
            assert_eq!(reopened.project.input, document.project.input);
            let unpacks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let observed = unpacks.clone();
            let reload = source_loading::SourceLoader::new_observed(move |_, phase, _| {
                if matches!(phase, source_loading::NativePhase::UnpackBegin) {
                    observed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                Ok(())
            });
            let reopened = load(&reopened, &reload);
            assert_eq!(
                unpacks.load(std::sync::atomic::Ordering::Relaxed),
                1,
                "cold reopen must actually redevelop the source, not reuse original leases"
            );
            let reopened_scan = reopened.context.scan().unwrap();
            let reopened_pixels = reopened_scan.preview_source(128).unwrap().2;
            assert!(
                reopened_pixels == *bases.last().unwrap(),
                "save/reopen must preserve developed camera pixels"
            );
            assert_eq!(reopened.context.reference, ready.reference);
            let area = scan.area;
            let export_size = [area.width.min(192), area.height.min(128)];
            let export_pixels = (export_size[0] * export_size[1]) as usize;
            let crop = Composition::with_crop(
                area,
                Default::default(),
                expected.orientation,
                Some(crate::crop::Crop {
                    center: [
                        f64::from(area.x) + f64::from(area.width) / 2.,
                        f64::from(area.y) + f64::from(area.height) / 2.,
                    ],
                    size: export_size,
                    angle: 0.,
                }),
            )
            .unwrap();
            for (extension, format) in [
                ("tif", crate::gpu_export::Format::Tiff(OutputTarget::Srgb)),
                (
                    "exr",
                    crate::gpu_export::Format::Exr(crate::export::ExrPrecision::Full),
                ),
            ] {
                let output = output_root.join(format!("{index}-{mode}.{extension}"));
                crate::gpu_export::write(
                    &renderer,
                    &device,
                    (&queue, &gate),
                    crate::gpu_export::Request {
                        scan: &reopened_scan,
                        field: None,
                        input: settings,
                        roll: document.project.roll,
                        grade: grade.clone(),
                        repairs: &[],
                        composition: crop,
                        format,
                        luts: Default::default(),
                        pipeline: Default::default(),
                    },
                    &output,
                    crate::export::Overwrite::Refuse,
                )
                .unwrap();
                if extension == "tif" {
                    let mut decoder =
                        tiff::decoder::Decoder::new(std::fs::File::open(&output).unwrap()).unwrap();
                    assert_eq!(
                        decoder.dimensions().unwrap(),
                        (crop.size()[0], crop.size()[1])
                    );
                    let values = crate::gpu_export::tests::tiff_pixels(&output);
                    assert_eq!(values.len(), export_pixels * 3);
                    assert!(values.windows(6).step_by(3).any(|p| p[..3] != p[3..]));
                } else {
                    let values = crate::gpu_export::tests::exr_pixels(&output);
                    assert_eq!(values.len(), export_pixels);
                    assert!(values.iter().flatten().all(|v| v.is_finite()));
                    assert!(values.windows(2).any(|p| p[0] != p[1]));
                }
            }
            reload.close_and_wait().unwrap();
            loader.close_and_wait().unwrap();
            eprintln!(
                "{} {mode}: {}x{}, area={:?}, import/reopen/view/TIFF/EXR passed",
                path.display(),
                header.width,
                header.height,
                scan.area
            );
        }
        let wb = header.as_shot_wb();
        if wb[..3].iter().any(|v| *v != wb[1]) {
            assert!(
                bases[0] != bases[1],
                "non-neutral as-shot WB must affect developed pixels"
            );
        }
        let after = std::fs::metadata(path).unwrap();
        assert_eq!(before.len(), after.len());
        assert_eq!(before.modified().unwrap(), after.modified().unwrap());
    }
}
