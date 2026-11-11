use super::*;
use crate::adjustment_layers::AdjustmentLayerSettings;
use crate::gpu::submission::SubmissionGate;
use crate::gpu_export::authored::{Request, write};
use crate::{
    adjustment_layers::{AdjustmentLayer, AdjustmentLayerCoverage, AdjustmentLayerPipeline},
    dust_repair::{Repair, RepairId, RepairOrigin, SupportMask},
    geometry::SourcePoint,
    processing::{Calibration, InvertedPixel},
};

#[test]
fn production_repairs_grades_look_and_area_match_independent_scalar_ocio_in_linear_target() {
    let (device, queue, renderer) = super::super::tests::context();
    let gate = SubmissionGate::default();
    let fixture_directory = tempfile::tempdir().unwrap();
    let mut bytes =
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"))
            .unwrap();
    assert_eq!(bytes.len(), 98_950, "declared authored single-strip source");
    let strip = bytes.len() - 256 * 192 * 2;
    for y in 94..98 {
        for x in 158..162 {
            let at = strip + (y * 256 + x) * 2;
            assert_eq!(
                u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap()),
                3264,
                "defect begins in the declared uniform plateau"
            );
            bytes[at..at + 2].copy_from_slice(&192_u16.to_le_bytes());
        }
    }
    let path = fixture_directory.path().join("marked.dng");
    std::fs::write(&path, bytes).unwrap();
    let scan = Scan::open(&path, None).unwrap();
    let a = scan.area;
    let composition = Composition::with_crop(
        a,
        Default::default(),
        Default::default(),
        Some(crate::crop::Crop {
            center: [
                f64::from(a.x) + f64::from(a.width) / 2.,
                f64::from(a.y) + f64::from(a.height) / 2.,
            ],
            size: [a.width - 96, a.height - 96],
            angle: 0.13,
        }),
    )
    .unwrap();
    let calibration = Calibration {
        dmin: DVec3::splat(0.8),
        dmax: DVec3::splat(0.02),
    };
    let roll = RollSettings::default();
    let grade = AdjustmentLayerSnapshot::new(vec![
        crate::adjustment_layers::AdjustmentLayer::global(AdjustmentLayerSettings {
            exposure: 0.5,
            contrast: 0.8,
            saturation: 1.1,
            wb: crate::color::Balance {
                warmth: 0.04,
                tint: -0.03,
            },
            ..Default::default()
        }),
        AdjustmentLayer {
            mask: AdjustmentLayerCoverage::Linear {
                start: SourcePoint([f64::from(a.x), f64::from(a.y)]),
                end: SourcePoint([f64::from(a.x + a.width), f64::from(a.y + a.height)]),
            },
            photo: AdjustmentLayerSettings {
                exposure: 0.5,
                ..Default::default()
            },
            enabled: true,
            invert: false,
            strength: 0.7,
        },
    ])
    .unwrap();
    let pipeline =
        AdjustmentLayerPipeline::new(calibration, roll, grade.clone(), scan.camera_to_ap1).unwrap();
    let centre = [160, 96];
    let repair = Repair {
        id: RepairId(1),
        enabled: true,
        origin: RepairOrigin::Manual,
        support: SupportMask::new([centre[0] - 8, centre[1] - 8], [16, 16], vec![true; 256])
            .unwrap(),
        offset: [64, 0],
    };
    // The scalar input is independent of GPU interpretation; healer fitting is shared.
    let evaluated = Arc::new(
        crate::repair_source::evaluate(
            &crate::scan::ImageSource::new(&scan, calibration).unwrap(),
            &repair,
        )
        .unwrap(),
    );
    let asset = Arc::new(
        crate::looks::LookAsset::new(
            "Stage oracle".into(),
            crate::looks::LookOrigin::Imported,
            Arc::new(
                crate::looks::Lut3D::new(
                    2,
                    [-2.; 3],
                    [2.; 3],
                    (0..8)
                        .map(|i| {
                            [
                                -1.8 + 4. * (i % 2) as f32,
                                -1.9 + 4. * ((i / 2) % 2) as f32,
                                -1.85 + 4. * (i / 4) as f32,
                            ]
                        })
                        .collect(),
                )
                .unwrap(),
            ),
        )
        .unwrap(),
    );
    let raster = FrameRaster::full(composition).unwrap();
    let frame = composition
        .source_to_frame(SourcePoint(centre.map(|v| f64::from(v) + 0.5)))
        .0
        .map(|v| v.floor() as u32);
    let edge = composition
        .source_to_frame(SourcePoint([
            f64::from(centre[0] + 8),
            f64::from(centre[1]),
        ]))
        .0
        .map(|v| v.floor() as u32);
    let size = composition.size();
    let points = [frame, edge, [1, 1], [size[0] - 2, size[1] - 2]];
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("not-opened.tif");
    let mut repair_effect = 0_f64;
    let mut look_effect = 0_f64;
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let cpu = crate::rendering::CpuRenderer::new(target).unwrap();
        let mut without_look: Option<DVec3> = None;
        for look in [None, Some(asset.clone())] {
            let request = Request {
                pipeline: Default::default(),
                scan: &scan,
                field: None,
                input: calibration.into(),
                roll,
                grade: grade.clone(),
                repairs: std::slice::from_ref(&repair),
                composition,
                format: Format::Tiff(target),
                luts: (look.clone()).into(),
            };
            let (result, pixels) = crate::export_timing::pixels::capture(|| {
                write(
                    &renderer,
                    &device,
                    (&queue, &gate),
                    request,
                    &path,
                    Overwrite::Refuse,
                )
            });
            result.unwrap();
            let codes = pixels.unwrap().codes;
            for (sample, &point) in points.iter().enumerate() {
                let cells = raster
                    .footprint(point)
                    .unwrap()
                    .pixels(a)
                    .unwrap()
                    .collect::<Vec<_>>();
                let working = cells
                    .iter()
                    .map(|(p, _)| {
                        let value = if repair.support.contains(*p) {
                            pipeline
                                .grade_neutral(
                                    InvertedPixel {
                                        rgb: DVec3::from_array(
                                            evaluated.pixel(*p, [0.; 3]).map(f64::from),
                                        ),
                                        floored: [false; 3],
                                    },
                                    *p,
                                )
                                .unwrap()
                                .rgb
                        } else {
                            pipeline
                                .apply(scan.pixel(p[0], p[1]).unwrap(), *p)
                                .unwrap()
                                .rgb
                        };
                        look.as_ref()
                            .map_or(Ok(value), |asset| asset.lut().apply(value))
                            .unwrap()
                    })
                    .collect::<Vec<_>>();
                let rendered = cpu.render(&working).unwrap();
                let expected = crate::sampling::weighted_mean_iter(
                    rendered
                        .iter()
                        .zip(&cells)
                        .map(|(value, (_, weight))| Ok((value.as_dvec3(), *weight))),
                )
                .unwrap();
                let actual = codes[(point[1] * size[0] + point[0]) as usize];
                for c in 0..3 {
                    let low = target
                        .decode(((f64::from(actual[c]) - 0.5) / 65535.).max(0.))
                        .unwrap();
                    let high = target
                        .decode(((f64::from(actual[c]) + 0.5) / 65535.).min(1.))
                        .unwrap();
                    assert!(
                        expected[c] >= low - 2e-5 && expected[c] <= high + 2e-5,
                        "target {target:?} look={} point={point:?} c={c} linear={} quantized interval=[{low},{high}]",
                        look.is_some(),
                        expected[c]
                    );
                }
                if sample == 0 {
                    let plain = cells
                        .iter()
                        .map(|(p, _)| {
                            let value = pipeline
                                .apply(scan.pixel(p[0], p[1]).unwrap(), *p)
                                .unwrap()
                                .rgb;
                            look.as_ref()
                                .map_or(Ok(value), |asset| asset.lut().apply(value))
                                .unwrap()
                        })
                        .collect::<Vec<_>>();
                    let plain = cpu.render(&plain).unwrap();
                    let plain = crate::sampling::weighted_mean_iter(
                        plain
                            .iter()
                            .zip(&cells)
                            .map(|(v, (_, w))| Ok((v.as_dvec3(), *w))),
                    )
                    .unwrap();
                    repair_effect = repair_effect.max((expected - plain).abs().max_element());
                    if let Some(base) = without_look {
                        look_effect = look_effect.max((expected - base).abs().max_element());
                    } else {
                        without_look = Some(expected);
                    }
                }
            }
        }
    }
    assert!(
        repair_effect > 0.01,
        "the marked defect must distinguish omitted healing: area={a:?}, centre={centre:?}, effect={repair_effect}, raw={:?}, healed={:?}",
        scan.pixel(centre[0], centre[1]).unwrap(),
        evaluated.pixel(centre, [0.; 3])
    );
    assert!(
        look_effect > 0.001,
        "fixture must distinguish look omission"
    );
}
