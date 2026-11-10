#![cfg(feature = "raw")]
use antitone::{raw::RawFile, scan::Scan};
use glam::DVec3;
use std::fs;

#[path = "support/fixture_path.rs"]
mod fixture_path;
#[path = "support/sony_ycc_fixture.rs"]
mod sony_ycc;

fn assert_light(actual: DVec3, expected: DVec3, gain: f64) {
    // YCbCr reconstruction, black subtraction and RGB16 gain each truncate to native
    // codes; the source restoration can amplify the final one-code truncation by gain.
    assert!(
        (actual - expected).abs().max_element() <= 2. * gain / 65535.,
        "{actual:?} != {expected:?}"
    );
}

#[test]
fn prebalanced_non_cfa_raw_preserves_camera_light_and_explicit_wb() {
    let fixture = sony_ycc::source();
    let path = fixture.path();
    let original = fs::read(path).unwrap();
    let mut raw = RawFile::open(path).unwrap();
    assert_eq!(raw.metadata.filters, 0);
    assert_eq!(raw.metadata.colors, 3);
    assert_eq!(raw.metadata.decoder(), "sony_ycbcr_load_raw()");
    assert_eq!(raw.metadata.as_shot_wb(), sony_ycc::WB);
    assert_eq!(raw.metadata.source_dimensions().unwrap(), [128, 64]);

    let unity = Scan::open(path, Some([1.; 4])).unwrap();
    let automatic = Scan::open(path, None).unwrap();
    // The unused fourth slot cannot control RGB normalization, even with an explicit gain.
    let custom_wb = [4096., 1024., 1536., 65536.];
    let custom = Scan::open(path, Some(custom_wb)).unwrap();
    for (index, y) in sony_ycc::LUMINANCE.into_iter().enumerate() {
        // Sony's decoder produces Y for neutral Cb/Cr, with black 1024 and white 17536.
        let light = f64::from(y - 1024) / (17536. - 1024.);
        let x = index as u32 * 64 + 32;
        assert_light(automatic.pixel(x, 32).unwrap(), DVec3::splat(light), 1.);
        assert_light(
            unity.pixel(x, 32).unwrap(),
            DVec3::new(light / 2., light, light * 2. / 3.),
            1.,
        );
        assert_light(
            custom.pixel(x, 32).unwrap(),
            DVec3::new(light * 2., light, light),
            2.,
        );
        // Repeated development must restore the decoder state, including baked-in WB.
        for (wb, expected) in [
            (sony_ycc::WB, DVec3::splat(light)),
            ([1.; 4], DVec3::new(light / 2., light, light * 2. / 3.)),
            (custom_wb, DVec3::new(light, light / 2., light / 2.)),
        ] {
            let developed = raw.develop_camera(wb).unwrap();
            assert_eq!(developed.maximum, 16512);
            assert_light(developed.pixel(x, 32).unwrap(), expected, 1.);
        }
    }
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn selected_ycbcr_raw_ifd_supplies_automatic_sony_shading() {
    let fixture = sony_ycc::source_with_profile(Some([2048, 1024, 1536]), true);
    let scan = Scan::open(fixture.path(), None).unwrap();
    assert!(scan.source().metadata().lens_identity().unwrap().is_some());
    let gain = 2_f64.sqrt();
    // Reciprocal exp2 profile decoding and the precision-scale product can round separately.
    let floor = gain / 65535.;
    assert!((scan.source().precision_floor() - floor).abs() <= 4. * f64::EPSILON * floor);
    for (index, y) in sony_ycc::LUMINANCE.into_iter().enumerate() {
        let light = f64::from(y - 1024) / (17536. - 1024.);
        assert_light(
            scan.pixel(index as u32 * 64 + 32, 32).unwrap(),
            DVec3::splat(light * gain),
            gain,
        );
    }
}

#[test]
fn non_cfa_admission_does_not_bypass_decode_failure() {
    let source = sony_ycc::source();
    let mut bytes = fs::read(source.path()).unwrap();
    // Damage the first tile's lossless JPEG marker while retaining an admissible TIFF header.
    let jpeg = bytes.windows(2).position(|b| b == [255, 216]).unwrap();
    bytes[jpeg] = 0;
    let broken = fixture_path::FixturePath::write("broken.arw", &bytes);
    let mut raw = RawFile::open(broken.path()).unwrap();
    assert!(raw.develop_camera(sony_ycc::WB).is_err());
}

#[test]
fn prebalanced_raw_without_recoverable_capture_balance_is_rejected() {
    let source = sony_ycc::source_with_balance(None);
    let error = RawFile::open(source.path())
        .err()
        .expect("missing capture WB");
    assert!(
        error
            .to_string()
            .contains("no usable as-shot white balance")
    );
}

#[cfg(all(feature = "gpu", feature = "export", feature = "project"))]
#[test]
fn prebalanced_camera_light_reaches_scene_linear_gpu_output() {
    use antitone::{
        color::ap1_to_ap0,
        export::{ExrPrecision, Overwrite},
        geometry::Composition,
        gpu::submission::SubmissionGate,
        gpu_export::{self, Format, Request},
        processing::InputSettings,
        renderer::GpuRenderer,
    };
    let (device, queue, renderer) = pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let renderer = GpuRenderer::new(&device, &queue).await.unwrap();
        (device, queue, renderer)
    });
    let fixture = sony_ycc::source();
    let directory = tempfile::tempdir().unwrap();
    for (index, (wb, camera_gains, source_gain)) in [
        (sony_ycc::WB, DVec3::ONE, 1.),
        ([4096., 1024., 1536., 65536.], DVec3::new(2., 1., 1.), 2.),
    ]
    .into_iter()
    .enumerate()
    {
        let scan = Scan::open(fixture.path(), Some(wb)).unwrap();
        let composition =
            Composition::new(scan.area, Default::default(), Default::default()).unwrap();
        let path = directory.path().join(format!("{index}.exr"));
        gpu_export::write(
            &renderer,
            &device,
            (&queue, &SubmissionGate::default()),
            Request {
                scan: &scan,
                field: None,
                input: InputSettings::Digital,
                roll: Default::default(),
                grade: antitone::adjustment_layers::AdjustmentLayerSnapshot::new(vec![]).unwrap(),
                repairs: &[],
                composition,
                format: Format::Exr(ExrPrecision::Full),
                luts: Default::default(),
                pipeline: Default::default(),
            },
            &path,
            Overwrite::Refuse,
        )
        .unwrap();
        let image = exr::prelude::read_first_flat_layer_from_file(path).unwrap();
        assert_eq!(image.layer_data.channel_data.list.len(), 3);
        let matrix = ap1_to_ap0() * scan.camera_to_ap1;
        for channel in &image.layer_data.channel_data.list {
            let c = match channel.name.to_string().as_str() {
                "R" => 0,
                "G" => 1,
                "B" => 2,
                _ => panic!("unexpected EXR channel"),
            };
            let exr::prelude::FlatSamples::F32(values) = &channel.sample_data else {
                panic!("full-precision scene output expected")
            };
            for (tile, y) in sony_ycc::LUMINANCE.into_iter().enumerate() {
                let light = f64::from(y - 1024) / (17536. - 1024.);
                let expected = (matrix * (camera_gains * light))[c];
                // Propagate the native two-code bound through the linear matrix, plus f32 roundoff.
                let allowance =
                    matrix.row(c).abs().element_sum() * 2. * source_gain / 65535. + 2e-6;
                let actual = f64::from(values[32 * 128 + tile * 64 + 32]);
                assert!(
                    (actual - expected).abs() <= allowance,
                    "{actual} != {expected}"
                );
            }
        }
    }
}

#[cfg(feature = "project")]
fn record(scan: &Scan, path: &std::path::Path) -> antitone::project::Source {
    use antitone::project::{Source, SourceDevelopment, SourceId};
    let metadata = scan.source().metadata();
    Source {
        id: SourceId(0),
        path: path.into(),
        development: SourceDevelopment {
            decode: metadata.development(scan.white_balance),
            dimensions: metadata.source_dimensions().unwrap(),
            active_area: [scan.area.x, scan.area.y, scan.area.width, scan.area.height],
            camera_model: scan.camera_model.clone(),
            camera_to_ap1: scan.camera_to_ap1.to_cols_array_2d(),
        },
        adjustment_layers: None,
        repairs: Default::default(),
        orientation: metadata.photo_orientation().unwrap(),
        crop: None,
    }
}

#[cfg(feature = "project")]
#[test]
fn accepted_wb_separates_cache_entries_with_the_same_geometry_basis_and_precision() {
    use antitone::source::{SourceCache, SourceKey};
    use std::sync::Arc;
    let fixture = sony_ycc::source();
    let unity = Scan::open(fixture.path(), Some([1.; 4])).unwrap();
    let digital = Scan::open(fixture.path(), None).unwrap();
    let unity_record = record(&unity, fixture.path());
    let digital_record = record(&digital, fixture.path());
    let mut without_balance = digital_record.development.clone();
    without_balance.decode.white_balance = unity_record.development.decode.white_balance;
    assert_eq!(unity_record.development, without_balance);
    let unity_key = SourceKey::capture(fixture.path(), &unity_record.development).unwrap();
    let digital_key = SourceKey::capture(fixture.path(), &digital_record.development).unwrap();
    assert_ne!(unity_key, digital_key);
    let cache = SourceCache::new(u64::MAX);
    cache
        .publish_source(unity_key.clone(), unity.source().clone())
        .unwrap()
        .unwrap();
    assert!(cache.get(&digital_key).is_none());
    assert!(
        cache
            .publish_source(digital_key.clone(), unity.source().clone())
            .is_err()
    );
    cache
        .publish_source(digital_key.clone(), digital.source().clone())
        .unwrap()
        .unwrap();
    assert!(Arc::ptr_eq(&cache.get(&unity_key).unwrap(), unity.source()));
    assert!(Arc::ptr_eq(
        &cache.get(&digital_key).unwrap(),
        digital.source()
    ));
}

#[cfg(feature = "project")]
#[test]
fn prebalanced_source_save_open_and_cold_development_retain_the_photographic_basis() {
    use antitone::{
        processing::{Calibration, InputSettings},
        project::{Development, Project, Save},
    };
    let fixture = sony_ycc::source();
    let path = fixture.path();
    let directory = tempfile::tempdir().unwrap();
    for (input, wb, scale) in [
        (InputSettings::Digital, sony_ycc::WB, 1.),
        (
            InputSettings::negative(Calibration {
                dmin: DVec3::splat(0.7),
                dmax: DVec3::splat(0.01),
            }),
            [1.; 4],
            1.,
        ),
    ] {
        let scan = Scan::open(path, Some(wb)).unwrap();
        let pixels = [scan.pixel(32, 32).unwrap(), scan.pixel(96, 32).unwrap()];
        let mut project = Project {
            input,
            ..Default::default()
        };
        let id = project.add_source(record(&scan, path)).unwrap();
        assert_eq!(scan.source().precision_floor(), scale / 65535.);
        let decode = &project.source(id).unwrap().development.decode;
        assert_eq!(
            Development::for_source(&project.source(id).unwrap().development),
            *decode
        );
        let saved = directory.path().join("prebalanced.antitone");
        project.save(&saved, Save::Replace).unwrap();
        drop(scan);
        let loaded = Project::load(&saved).unwrap();
        loaded.require_runtime().unwrap();
        let source = loaded.source(id).unwrap();
        assert_eq!(source, project.source(id).unwrap());
        let cold = Scan::open(&source.path, Some(source.development.decode.white_balance)).unwrap();
        let recipe = loaded.image_recipe(id).unwrap();
        recipe.neutral.require_scan(&cold).unwrap();
        assert_eq!(
            [cold.pixel(32, 32).unwrap(), cold.pixel(96, 32).unwrap()],
            pixels
        );
        let json = std::fs::read_to_string(&saved).unwrap();
        assert!(!json.contains("pre_applied_wb"));
        assert_eq!(source.development.decode.white_balance, wb);
        let wrong_stage = Scan::open(
            path,
            if input.is_negative() {
                None
            } else {
                Some([1.; 4])
            },
        )
        .unwrap();
        assert!(recipe.neutral.require_scan(&wrong_stage).is_err());
    }
}
