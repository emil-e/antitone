#![cfg(all(feature = "raw", feature = "project", target_os = "macos"))]

use std::fs;

#[path = "support/sony_fixture.rs"]
mod fixture;
#[cfg(all(feature = "gpu", feature = "export", feature = "ocio-reference"))]
#[path = "support/working_precision.rs"]
mod working_precision;
use fixture::{Entry, Order, profile_tags, source};

use antitone::{
    lens::LensCorrections,
    raw::{RawFile, SonyLensDataStatus},
};
use glam::DVec3;

#[path = "support/sony_vignetting_raw.rs"]
mod vignetting;

#[test]
fn selected_raw_ifd_not_rendered_ifd_supplies_signed_coefficients_in_either_byte_order() {
    for order in [Order::Little, Order::Big] {
        let file = source(order, profile_tags(order), false, true);
        let bytes = std::fs::read(file.path()).unwrap();
        let raw = RawFile::open(file.path()).unwrap();
        let metadata = raw.metadata.sony_lens;
        assert_eq!(
            metadata.distortion_status(),
            SonyLensDataStatus::Available {
                camera_setting: Some(0)
            }
        );
        assert_eq!(
            metadata.ca_status(),
            SonyLensDataStatus::Available {
                camera_setting: Some(1)
            }
        );
        let profile = metadata.profile().unwrap().unwrap();
        let scales = profile.inverse_scales(1., LensCorrections::ALL).unwrap();
        assert_eq!(scales, DVec3::new(129. / 128., 1., 63. / 64.) * (15. / 16.));
        assert_eq!(std::fs::read(file.path()).unwrap(), bytes);
    }
}

#[test]
fn tiled_raw_selection_uses_the_offset_table_not_its_first_image_offset() {
    for order in [Order::Little, Order::Big] {
        let mut tags = profile_tags(order);
        tags.extend([
            (322, 4, order.long(32)),
            (323, 4, order.long(64)),
            (324, 4, [order.long(0), order.long(0)].concat()),
            (325, 4, [order.long(4096), order.long(4096)].concat()),
        ]);
        let file = source(order, tags, false, true);
        let raw = RawFile::open(file.path()).unwrap();
        assert!(
            raw.metadata
                .sony_lens
                .profile()
                .unwrap()
                .unwrap()
                .has_distortion()
        );
    }
}

#[test]
fn diagnostic_bypass_keeps_original_codes_and_capture_diagnostics() {
    let order = Order::Little;
    let file = source(order, profile_tags(order), false, false);
    let plain_file = source(order, vec![], false, false);
    let mut raw = RawFile::open_uncorrected(file.path()).unwrap();
    let mut plain = RawFile::open_uncorrected(plain_file.path()).unwrap();
    assert!(raw.metadata.sony_lens.profile().unwrap().unwrap().has_ca());
    let before = raw.unpack().unwrap();
    assert_eq!(before.at_white, [0; 4]);
    let developed = raw.develop_camera([1.; 4]).unwrap();
    let baseline = plain.develop_camera([1.; 4]).unwrap();
    assert_eq!(
        (developed.width, developed.height),
        (baseline.width, baseline.height)
    );
    assert_ne!(
        developed.pixel_codes(16, 16).unwrap(),
        developed.pixel_codes(48, 48).unwrap()
    );
    for y in 0..developed.height {
        for x in 0..developed.width {
            assert_eq!(
                developed.pixel_codes(x, y).unwrap(),
                baseline.pixel_codes(x, y).unwrap()
            );
        }
    }
    assert_eq!(raw.unpack().unwrap(), before);
}

#[test]
fn unavailable_flags_override_leftover_payloads_but_off_does_not() {
    let order = Order::Little;
    let tags = profile_tags(order)
        .into_iter()
        .map(|(tag, kind, data)| {
            if tag == 0x7034 || tag == 0x7036 {
                (tag, kind, order.short(255))
            } else {
                (tag, kind, data)
            }
        })
        .collect();
    let file = source(order, tags, false, false);
    let raw = RawFile::open(file.path()).unwrap();
    assert_eq!(
        raw.metadata.sony_lens.distortion_status(),
        SonyLensDataStatus::Unavailable
    );
    assert_eq!(
        raw.metadata.sony_lens.ca_status(),
        SonyLensDataStatus::Unavailable
    );
    assert!(raw.metadata.sony_lens.profile().unwrap().is_none());
}

#[test]
fn an_unavailable_component_does_not_disable_the_other_and_zero_profiles_remain_identifiable() {
    let order = Order::Little;
    let tags = vec![
        (0x7036, 3, order.short(255)),
        (0x7037, 8, order.signed(&[0])),
        (0x7035, 8, order.signed(&[4, 100, 100, -100, -100])),
    ];
    let file = source(order, tags, false, false);
    let raw = RawFile::open(file.path()).unwrap();
    let profile = raw.metadata.sony_lens.profile().unwrap().unwrap();
    assert!(!profile.has_distortion() && profile.has_ca());

    let file = source(
        order,
        vec![(0x7037, 8, order.signed(&[2, 0, 0]))],
        false,
        false,
    );
    let raw = RawFile::open(file.path()).unwrap();
    assert_eq!(
        raw.metadata.sony_lens.distortion_status(),
        SonyLensDataStatus::Available {
            camera_setting: None
        }
    );
    let profile = raw.metadata.sony_lens.profile().unwrap().unwrap();
    assert!(!profile.has_distortion() && !profile.has_ca());
}

#[test]
fn ambiguous_raw_ifds_with_the_same_storage_do_not_choose_arbitrary_coefficients() {
    let order = Order::Little;
    let file = source(order, profile_tags(order), false, true);
    let mut bytes = std::fs::read(file.path()).unwrap();
    let short = |at| u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap());
    let first_count = short(8) as usize;
    let next = 10 + first_count * 12;
    let second = u32::from_le_bytes(bytes[next..next + 4].try_into().unwrap()) as usize;
    let first_strip = (0..first_count)
        .map(|i| 10 + i * 12)
        .find(|&at| short(at) == 273)
        .unwrap();
    let strip = bytes[first_strip + 8..first_strip + 12].to_vec();
    let count = short(second) as usize;
    for i in 0..count {
        let at = second + 2 + i * 12;
        let tag = u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap());
        if tag == 262 {
            bytes[at + 8..at + 10].copy_from_slice(&order.short(32803));
        }
        if tag == 273 {
            bytes[at + 8..at + 12].copy_from_slice(&strip);
        }
    }
    std::fs::write(file.path(), bytes).unwrap();
    assert!(RawFile::open(file.path()).is_err());
    let raw = RawFile::open_uncorrected(file.path()).unwrap();
    assert!(raw.metadata.sony_lens.profile().is_err());
}

#[test]
fn missing_data_and_converted_dng_do_not_inherit_rendered_or_sony_profiles() {
    for dng in [false, true] {
        let order = Order::Little;
        let tags = if dng { profile_tags(order) } else { vec![] };
        let file = source(order, tags, dng, true);
        let raw = RawFile::open(file.path()).unwrap();
        assert_eq!(
            raw.metadata.sony_lens.distortion_status(),
            SonyLensDataStatus::Absent
        );
        assert_eq!(
            raw.metadata.sony_lens.ca_status(),
            SonyLensDataStatus::Absent
        );
        assert!(raw.metadata.sony_lens.profile().unwrap().is_none());
    }
}

#[test]
fn malformed_available_profiles_fail_development_but_remain_diagnosable() {
    let order = Order::Little;
    for tags in [
        vec![(0x7036, 4, order.long(0))],
        vec![(0x7037, 3, order.short(2))],
        vec![(0x7037, 8, order.signed(&[0; 18]))],
        vec![(0x7037, 8, order.signed(&[16, 0, 0]))],
        vec![
            (0x7037, 8, order.signed(&[2, 0, 0])),
            (0x7037, 8, order.signed(&[2, 0, 0])),
        ],
    ] {
        let file = source(order, tags, false, false);
        assert!(RawFile::open(file.path()).is_err());
        let raw = RawFile::open_uncorrected(file.path()).unwrap();
        assert!(raw.metadata.sony_lens.profile().is_err());
    }
}

#[test]
fn automatic_correction_matches_independent_bilinear_camera_samples_and_repeats() {
    let order = Order::Little;
    let file = source(order, profile_tags(order), false, false);
    let mut original = RawFile::open_uncorrected(file.path()).unwrap();
    let mut corrected = RawFile::open(file.path()).unwrap();
    assert!(corrected.metadata.lens_identity().unwrap().is_some());
    assert_eq!(corrected.metadata.source_dimensions().unwrap(), [64, 64]);
    let capture = corrected.unpack().unwrap();
    let baseline = original.develop_camera([1.; 4]).unwrap();
    let samples = {
        let image = corrected.develop_camera([1.; 4]).unwrap();
        assert!(image.pixel_codes(10, 20).is_err());
        let mut samples = Vec::new();
        let mut moved = false;
        for [x, y] in [[0, 0], [10, 20], [37, 41], [63, 63]] {
            let actual = image.pixel_samples(x, y).unwrap();
            for (c, factor) in [
                (129. / 128.) * (15. / 16.),
                15. / 16.,
                (63. / 64.) * (15. / 16.),
            ]
            .into_iter()
            .enumerate()
            {
                let sx = 31.5 + (x as f64 - 31.5) * factor;
                let sy = 31.5 + (y as f64 - 31.5) * factor;
                let left = sx.floor() as u32;
                let top = sy.floor() as u32;
                let u = sx - left as f64;
                let v = sy - top as f64;
                let p = |dx, dy| f64::from(baseline.pixel_codes(left + dx, top + dy).unwrap()[c]);
                let expected = p(0, 0) * (1. - u) * (1. - v)
                    + p(1, 0) * u * (1. - v)
                    + p(0, 1) * (1. - u) * v
                    + p(1, 1) * u * v;
                assert_eq!(actual[c].to_bits(), (expected as f32).to_bits());
                moved |= actual[c] != f32::from(baseline.pixel_codes(x, y).unwrap()[c]);
            }
            samples.push(([x, y], actual));
        }
        assert!(
            moved,
            "the spatial fixture must distinguish a corrected source from a no-op"
        );
        samples
    };
    assert_eq!(corrected.unpack().unwrap(), capture);
    assert_eq!(
        corrected
            .region_sensor_diagnostics(antitone::raw::Rect {
                x: 0,
                y: 0,
                width: 64,
                height: 64
            })
            .unwrap(),
        capture,
        "detailed diagnostics retain their original native coordinates"
    );
    let repeated = corrected.develop_camera([1.; 4]).unwrap();
    for ([x, y], expected) in samples {
        assert_eq!(repeated.pixel_samples(x, y).unwrap(), expected);
    }
}

#[test]
fn foreground_and_background_reconstruction_cover_partial_row_chunks_identically() {
    use antitone::raw::DevelopmentPriority;
    let mut tags = profile_tags(Order::Little);
    tags.iter_mut().find(|tag| tag.0 == 0x7037).unwrap().2 = Order::Little.signed(&[2, 1024, 1024]);
    let file = source(Order::Little, tags, false, false);
    let mut expected = None;
    for priority in [
        DevelopmentPriority::Foreground,
        DevelopmentPriority::Background,
    ] {
        let mut raw = RawFile::open(file.path()).unwrap();
        raw.set_development_priority(priority);
        let image = raw.develop_camera([1.; 4]).unwrap();
        assert_eq!([image.width, image.height], [58, 58]);
        let mut pixels = Vec::new();
        for y in 0..image.height {
            for x in 0..image.width {
                let p = image.pixel_samples(x, y).unwrap();
                assert!(p[..3].iter().all(|&v| v > 0. && v <= 65535.));
                pixels.push(p);
            }
        }
        if let Some(expected) = &expected {
            assert_eq!(&pixels, expected);
        } else {
            expected = Some(pixels);
        }
    }
}

#[test]
fn scan_consumers_keep_continuous_values_and_original_capture_authority() {
    let file = source(Order::Little, profile_tags(Order::Little), false, false);
    let mut scan = antitone::scan::Scan::open(file.path(), Some([1.; 4])).unwrap();
    let (width, height, preview) = scan.preview_source(64).unwrap();
    assert_eq!((width, height), (64, 64));
    let tile = scan.tile_region(scan.area, 0, 0, width, height).unwrap();
    assert_eq!(preview, tile);
    assert!(
        preview
            .iter()
            .flat_map(|p| &p[..3])
            .any(|v| v.fract() != 0.)
    );
    for [x, y] in [[10, 20], [31, 40]] {
        let p = preview[(y * width + x) as usize];
        let expected = DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64) / 65535.;
        assert_eq!(scan.pixel(x, y).unwrap(), expected);
        assert_eq!(
            scan.reference(antitone::raw::Rect {
                x,
                y,
                width: 1,
                height: 1
            })
            .unwrap(),
            expected
        );
    }
    let samples = scan
        .analysis_samples(scan.area, 16, || false)
        .unwrap()
        .unwrap();
    assert!(!samples.pixels.is_empty());
    assert_eq!(samples.rejected_capture, 0);
}

#[test]
fn resampled_picks_still_reject_clipped_original_cfa_support() {
    let file = source(Order::Little, profile_tags(Order::Little), false, false);
    let mut bytes = fs::read(file.path()).unwrap();
    let count = u16::from_le_bytes(bytes[8..10].try_into().unwrap()) as usize;
    let entry = (0..count)
        .map(|i| 10 + 12 * i)
        .find(|&i| u16::from_le_bytes(bytes[i..i + 2].try_into().unwrap()) == 273)
        .unwrap();
    let offset = u32::from_le_bytes(bytes[entry + 8..entry + 12].try_into().unwrap()) as usize;
    let site = offset + 2 * (21 * 64 + 12);
    bytes[site..site + 2].copy_from_slice(&16383u16.to_le_bytes());
    fs::write(file.path(), bytes).unwrap();
    let mut scan = antitone::scan::Scan::open(file.path(), Some([1.; 4])).unwrap();
    let point = antitone::raw::Rect {
        x: 10,
        y: 20,
        width: 1,
        height: 1,
    };
    let diagnostics = scan.source().capture_eligibility(point).unwrap();
    assert!(diagnostics.at_white);
    assert!(
        scan.source()
            .region_sensor_diagnostics(point)
            .unwrap()
            .at_white
            .iter()
            .all(|&n| n == 0),
        "detailed diagnostics use original coordinates, not corrected eligibility support"
    );
    let readers: Vec<_> = (0..4)
        .map(|_| {
            let source = scan.source().clone();
            std::thread::spawn(move || source.capture_eligibility(point).unwrap())
        })
        .collect();
    for reader in readers {
        assert_eq!(reader.join().unwrap(), diagnostics);
    }
    assert!(scan.pixel(10, 20).unwrap().max_element() < 1.);
    assert!(
        scan.reference(point).is_err(),
        "interpolating away saturation must not make the capture eligible"
    );
    let samples = scan
        .analysis_samples(
            antitone::raw::Rect {
                x: 6,
                y: 16,
                width: 8,
                height: 8,
            },
            1,
            || false,
        )
        .unwrap()
        .unwrap();
    assert!(samples.pixels.is_empty());
    assert_eq!(samples.rejected_capture, 1);
}

#[cfg(feature = "project")]
fn development(scan: &antitone::scan::Scan) -> antitone::project::SourceDevelopment {
    antitone::project::SourceDevelopment {
        lens_correction: scan.source().metadata().lens_identity().unwrap(),
        dng_development: scan.source().metadata().dng_identity(),
        dimensions: scan.source().metadata().source_dimensions().unwrap(),
        active_area: [scan.area.x, scan.area.y, scan.area.width, scan.area.height],
        camera_model: scan.camera_model.clone(),
        camera_to_ap1: scan.camera_to_ap1.to_cols_array_2d(),
        input_floor: scan.source().input_floor(),
    }
}

#[cfg(feature = "project")]
#[test]
fn save_open_retains_lens_identity_and_flat_references_reject_different_maps_at_equal_dimensions() {
    use antitone::{
        flat_field::FlatField,
        project::{Development, FlatFieldSettings, Project, Save, Source, SourceId},
    };
    use std::sync::Arc;
    let file = source(Order::Little, profile_tags(Order::Little), false, false);
    let mut changed = profile_tags(Order::Little);
    changed.iter_mut().find(|tag| tag.0 == 0x7037).unwrap().2 =
        Order::Little.signed(&[2, -512, -512]);
    let other_file = source(Order::Little, changed, false, false);
    let mut scan = antitone::scan::Scan::open(file.path(), Some([1.; 4])).unwrap();
    let mut other = antitone::scan::Scan::open(other_file.path(), Some([1.; 4])).unwrap();
    let first_development = development(&scan);
    let second_development = development(&other);
    assert_ne!(
        first_development.lens_correction,
        second_development.lens_correction
    );
    let mut same_without_identity = second_development.clone();
    same_without_identity.lens_correction = first_development.lens_correction.clone();
    assert_eq!(
        same_without_identity, first_development,
        "only lens provenance differs in this fixture"
    );
    let mut project = Project {
        development: Some(Development::new(antitone::raw::version(), scan.fixed_wb)),
        dmin: Some([0.7; 3]),
        dmax: Some([0.01; 3]),
        input_floor: Some(scan.input_floor),
        ..Default::default()
    };
    let id = project
        .add_source(Source {
            id: SourceId(0),
            path: file.path().into(),
            reference_only: false,
            development: first_development,
            photo: Default::default(),
            orientation: Default::default(),
            crop: None,
            repairs: Default::default(),
            local_adjustments: None,
        })
        .unwrap();
    let map = Arc::new(
        FlatField::from_nodes(
            [scan.area.x, scan.area.y, scan.area.width, scan.area.height],
            [2, 2],
            vec![DVec3::ONE; 4],
        )
        .unwrap(),
    );
    let field = FlatFieldSettings::new(
        project.source(id).unwrap(),
        map.clone(),
        project.development.as_ref().unwrap(),
    );
    assert!(field.require_source(&second_development).is_err());
    scan.set_flat_field(Some(map.clone())).unwrap();
    other.set_flat_field(Some(map)).unwrap();
    project.flat_field = Some(field);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("sony.antitone");
    project.save(&path, Save::New).unwrap();
    let reopened = Project::load(&path).unwrap();
    reopened.require_runtime(&antitone::raw::version()).unwrap();
    let recipe = reopened.image_recipe(id).unwrap();
    assert_eq!(recipe, project.image_recipe(id).unwrap());
    recipe.neutral.require_scan(&scan).unwrap();
    assert!(recipe.neutral.require_scan(&other).is_err());
    let mut missing = recipe.clone();
    missing.neutral.source_development.lens_correction = None;
    assert!(missing.neutral.require_scan(&scan).is_err());
    let mut old = reopened;
    old.flat_field = None;
    old.development.as_mut().unwrap().contract =
        "libraw-camera-ahd-u16-linear-camera-inversion-ap1-bradford-code-floor-v2".into();
    assert!(old.require_runtime(&antitone::raw::version()).is_err());
}

#[cfg(all(feature = "gpu", feature = "export", feature = "ocio-reference"))]
#[test]
fn corrected_tiff_and_ap0_exr_match_scalar_original_samples_with_and_without_flat_field() {
    use antitone::{
        export::{self, ExrPrecision, Overwrite, ProcessingMetadata},
        flat_field::FlatField,
        geometry::{Composition, PhotoOrientation, RollOrientation},
        gpu::{GpuFlatField, SubmissionGate},
        gpu_export::{self, Format, Request},
        local_adjustments::{PhotoGradeSnapshot, SpatialPipeline},
        processing::{Calibration, PhotoSettings, RollSettings},
        renderer::GpuRenderer,
        rendering::OutputTarget,
    };
    use exr::prelude::{FlatSamples, read_first_flat_layer_from_file};
    use std::sync::Arc;
    let (device, queue, renderer) = pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let renderer = GpuRenderer::new(&device, &queue).await.unwrap();
        (device, queue, renderer)
    });
    let directory = tempfile::tempdir().unwrap();
    for (shading, flat) in [(false, false), (false, true), (true, false), (true, true)] {
        let mut tags = profile_tags(Order::Little);
        if shading {
            tags.push((0x7032, 8, Order::Little.signed(&[2, 8192, 8192])));
        }
        let file = source(Order::Little, tags, false, false);
        let mut scan = antitone::scan::Scan::open(file.path(), Some([1.; 4])).unwrap();
        let field = if flat {
            let field = Arc::new(
                FlatField::from_nodes(
                    [scan.area.x, scan.area.y, scan.area.width, scan.area.height],
                    [2, 2],
                    vec![DVec3::new(0.9, 1.1, 1.2); 4],
                )
                .unwrap(),
            );
            scan.set_flat_field(Some(field.clone())).unwrap();
            Some(Arc::new(GpuFlatField::new(&device, field).unwrap()))
        } else {
            None
        };
        let calibration = Calibration {
            dmin: DVec3::splat(0.7),
            dmax: DVec3::splat(0.01),
            input_floor: scan.input_floor,
        };
        let roll = RollSettings::new_grade();
        let grade = PhotoGradeSnapshot::new(
            PhotoSettings {
                contrast: 1.1,
                ..Default::default()
            },
            Vec::new(),
        )
        .unwrap();
        let composition = Composition::new(
            scan.area,
            RollOrientation::default(),
            PhotoOrientation::default(),
        )
        .unwrap();
        let scalar = SpatialPipeline::new(
            scan.processing_calibration(calibration).unwrap(),
            roll,
            grade.clone(),
            scan.camera_to_ap1,
        )
        .unwrap();
        for format in [
            Format::Tiff(OutputTarget::Srgb),
            Format::Exr(ExrPrecision::Full),
        ] {
            let actual = directory.path().join("gpu-output");
            let expected = directory.path().join("scalar-output");
            gpu_export::write(
                &renderer,
                &device,
                (&queue, &SubmissionGate::default()),
                Request {
                    scan: &scan,
                    field: field.clone(),
                    calibration,
                    roll,
                    grade: grade.clone(),
                    repairs: &[],
                    composition,
                    format,
                    look: None,
                },
                &actual,
                Overwrite::Confirmed,
            )
            .unwrap();
            let metadata = ProcessingMetadata::with_grade(roll.response, &grade);
            match format {
                Format::Tiff(target) => {
                    export::tiff_composed(
                        &expected,
                        composition,
                        target,
                        Overwrite::Confirmed,
                        metadata,
                        |x, y| scalar.apply(scan.pixel(x, y)?, [x, y]).map(|p| p.rgb),
                    )
                    .unwrap();
                    let decode = |path| {
                        let mut decoder =
                            tiff::decoder::Decoder::new(fs::File::open(path).unwrap()).unwrap();
                        assert_eq!(decoder.dimensions().unwrap(), (64, 64));
                        let tiff::decoder::DecodingResult::U16(values) =
                            decoder.read_image().unwrap()
                        else {
                            panic!("not RGB16")
                        };
                        values
                    };
                    assert!(
                        decode(&actual)
                            .iter()
                            .zip(decode(&expected))
                            .all(|(a, b)| a.abs_diff(b) <= 1)
                    );
                }
                Format::Exr(precision) => {
                    export::exr_composed(
                        &expected,
                        composition,
                        precision,
                        Overwrite::Confirmed,
                        metadata,
                        |x, y| scalar.apply(scan.pixel(x, y)?, [x, y]).map(|p| p.rgb),
                    )
                    .unwrap();
                    let actual = read_first_flat_layer_from_file(actual).unwrap();
                    let expected = read_first_flat_layer_from_file(expected).unwrap();
                    assert_eq!(actual.layer_data.size, expected.layer_data.size);
                    for (a, b) in actual
                        .layer_data
                        .channel_data
                        .list
                        .iter()
                        .zip(&expected.layer_data.channel_data.list)
                    {
                        assert_eq!(a.name, b.name);
                        let (FlatSamples::F32(a), FlatSamples::F32(b)) =
                            (&a.sample_data, &b.sample_data)
                        else {
                            panic!("not full precision")
                        };
                        assert_eq!(a.len(), b.len());
                        for (&a, &b) in a.iter().zip(b) {
                            let expected = b as f64;
                            assert!(
                                (a as f64 - expected).abs()
                                    <= working_precision::allowance(expected)
                                        + f32::EPSILON as f64 * expected.abs(),
                                "flat={flat}: actual={a} expected={expected}"
                            );
                        }
                    }
                }
            }
        }
    }
}
