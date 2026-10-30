use super::film_tests::{exr_pixels, tiff_pixels};
use super::*;
use crate::{
    adjustment_layers::{AdjustmentLayer, AdjustmentLayerCoverage, AdjustmentLayerSettings},
    geometry::SourcePoint,
    halation::{HalationSettings, reference},
};

#[test]
fn reopened_tiff_and_ap0_exr_include_independent_halation_after_the_full_grade() {
    let (device, queue, renderer) = super::tests::context();
    let scan = Scan::open(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        None,
    )
    .unwrap();
    let a = scan.area;
    let composition = Composition::new(a, Default::default(), Default::default()).unwrap();
    let halo = HalationSettings {
        enabled: true,
        size: 0.4,
        strength: 0.4,
    };
    let grade = AdjustmentLayerSnapshot::new(vec![
        AdjustmentLayer::global(AdjustmentLayerSettings {
            exposure: 0.4,
            ..Default::default()
        }),
        AdjustmentLayer {
            mask: AdjustmentLayerCoverage::Radial {
                center: SourcePoint([
                    f64::from(a.x) + f64::from(a.width) / 2.,
                    f64::from(a.y) + f64::from(a.height) / 2.,
                ]),
                radii: [48., 48.],
                angle: 0.,
                feather: 0.,
            },
            photo: AdjustmentLayerSettings {
                exposure: 2.,
                ..Default::default()
            },
            enabled: true,
            invert: false,
            strength: 0.8,
        },
    ])
    .unwrap();
    let matrix = scan.positive_matrix(&InputSettings::Digital).unwrap();
    // Exposure-only grading has an analytic gain: 0.4 EV outside the hard circle,
    // 0.4 + 2 × 0.8 = 2 EV inside. The varying gain does not commute with halation.
    let gain = |[x, y]: [u32; 2]| {
        let dx = f64::from(x) + 0.5 - (f64::from(a.x) + f64::from(a.width) / 2.);
        let dy = f64::from(y) + 0.5 - (f64::from(a.y) + f64::from(a.height) / 2.);
        if dx * dx + dy * dy < 48_f64.powi(2) {
            4.
        } else {
            2_f64.powf(0.4)
        }
    };
    let coordinates: Vec<_> = (a.y..a.y + a.height)
        .flat_map(|y| (a.x..a.x + a.width).map(move |x| [x, y]))
        .collect();
    let upstream: Vec<_> = coordinates
        .iter()
        .map(|&p| {
            let rgb = matrix * scan.pixel(p[0], p[1]).unwrap() * gain(p);
            [rgb.x, rgb.y, rgb.z, 0.]
        })
        .collect();
    let sigma = halo
        .parameters(composition.crop().size)
        .unwrap()
        .unwrap()
        .sigma;
    let expected = reference::apply(
        &upstream,
        [a.width as usize, a.height as usize],
        [sigma; 2],
        halo.strength,
        0,
    );
    assert!(
        upstream
            .iter()
            .zip(&expected)
            .any(|(a, b)| (a[0] - b[0]).abs() > 0.001),
        "fixture exposes halo omission"
    );
    let neutral: Vec<_> = coordinates
        .iter()
        .map(|&p| {
            let rgb = matrix * scan.pixel(p[0], p[1]).unwrap();
            [rgb.x, rgb.y, rgb.z, 0.]
        })
        .collect();
    let wrong = reference::apply(
        &neutral,
        [a.width as usize, a.height as usize],
        [sigma; 2],
        halo.strength,
        0,
    );
    let wrong_order_delta = coordinates
        .iter()
        .zip(wrong)
        .zip(&expected)
        .map(|((&p, v), e)| {
            let rgb = DVec3::new(v[0], v[1], v[2]) * gain(p);
            (rgb - DVec3::new(e[0], e[1], e[2])).abs().max_element()
        })
        .fold(0., f64::max);
    assert!(
        wrong_order_delta > 0.001,
        "source-anchored exposure must distinguish wrong order: delta {wrong_order_delta}, area {a:?}, sigma {sigma}"
    );
    let directory = tempfile::tempdir().unwrap();
    let gate = SubmissionGate::default();
    let write_file = |name: &str, format, settings, look| {
        let path = directory.path().join(name);
        write(
            &renderer,
            &device,
            (&queue, &gate),
            Request {
                scan: &scan,
                field: None,
                input: InputSettings::Digital,
                roll: Default::default(),
                grade: grade.clone(),
                repairs: &[],
                composition,
                format,
                look,
                film_emulation: Default::default(),
                halation: settings,
                film_seed: 17,
            },
            &path,
            Overwrite::Refuse,
        )
        .unwrap();
        path
    };
    let pixel = |x: u32, y: u32| {
        let p = expected[((y - a.y) * a.width + x - a.x) as usize];
        Ok(DVec3::new(p[0], p[1], p[2]))
    };
    for precision in [ExrPrecision::Half, ExrPrecision::Full] {
        let actual = write_file(
            &format!("halo-{precision:?}.exr"),
            Format::Exr(precision),
            halo,
            None,
        );
        let reference = directory
            .path()
            .join(format!("reference-{precision:?}.exr"));
        export::exr_composed(&reference, composition, precision, Overwrite::Refuse, pixel).unwrap();
        for (a, b) in exr_pixels(&actual).iter().zip(exr_pixels(&reference)) {
            for c in 0..3 {
                let tolerance = if precision == ExrPrecision::Half {
                    0.001
                } else {
                    2e-5
                };
                assert!(
                    (a[c] - b[c]).abs() <= tolerance * b[c].abs().max(1.),
                    "{precision:?}: {a:?} vs {b:?}"
                );
            }
        }
    }
    let actual = write_file("halo.tif", Format::Tiff(OutputTarget::Srgb), halo, None);
    let reference = directory.path().join("reference.tif");
    export::tiff_composed(
        &reference,
        composition,
        OutputTarget::Srgb,
        Overwrite::Refuse,
        pixel,
    )
    .unwrap();
    let actual_codes = tiff_pixels(&actual);
    let reference_codes = tiff_pixels(&reference);
    assert_eq!(actual_codes.len(), reference_codes.len());
    let maximum = actual_codes
        .iter()
        .zip(&reference_codes)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap();
    assert!(
        maximum <= 4,
        "f32 stage + independent OCIO/encoding: max code error {maximum}"
    );
    let clean = write_file(
        "clean.tif",
        Format::Tiff(OutputTarget::Srgb),
        Default::default(),
        None,
    );
    assert_ne!(
        actual_codes,
        tiff_pixels(&clean),
        "halation remains active with film emulation disabled"
    );
    let mut disabled = halo;
    disabled.enabled = false;
    let bypass = write_file(
        "disabled.tif",
        Format::Tiff(OutputTarget::Srgb),
        disabled,
        None,
    );
    assert_eq!(tiff_pixels(&clean), tiff_pixels(&bypass));
}
