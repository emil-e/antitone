use super::*;
use crate::local_adjustments::SpatialPipeline;
use tiff::decoder::{Decoder, DecodingResult};

#[test]
fn common_worker_preserves_fractional_above_white_sony_sources_with_residual_correction() {
    use crate::raw::sony_fixture::{Order, profile_tags, source};
    let order = Order::Little;
    let mut tags = profile_tags(order);
    tags.push((0x7032, 8, order.signed(&[2, 16384, 16384])));
    let file = source(order, tags, false, false);
    let mut scan = Scan::open(file.path(), Some([1.; 4])).unwrap();
    assert!(scan.source().camera_image().unwrap().is_continuous());
    let a = scan.area;
    let codes = scan.tile_region(a, 0, 0, a.width, a.height).unwrap();
    assert!(
        codes.iter().any(|p| p[..3].iter().any(|&v| v > 65535.)),
        "source headroom must be exercised"
    );
    assert!(
        codes.iter().any(|p| p[..3].iter().any(|&v| v != v.floor())),
        "source fractions must be exercised"
    );
    let field = Arc::new(
        crate::flat_field::FlatField::from_nodes(
            [a.x, a.y, a.width, a.height],
            [2, 2],
            vec![
                DVec3::splat(1.),
                DVec3::new(1.05, 1.1, 1.),
                DVec3::new(1.1, 1., 1.05),
                DVec3::splat(1.1),
            ],
        )
        .unwrap(),
    );
    scan.set_flat_field(Some(field.clone())).unwrap();
    let (device, queue, renderer) = super::super::super::tests::context();
    let gate = SubmissionGate::default();
    let gpu_field = Arc::new(GpuFlatField::new(&device, field).unwrap());
    let (mut project, id) = fixture(&scan);
    // The shaded source exceeds nominal white; keep the response off the black plateau.
    project.set_dmin(Some([4.; 3]));
    project.set_dmax(Some([0.05; 3]));
    project.source_mut(id).unwrap().crop = Some(crate::crop::Crop {
        center: [
            f64::from(a.x) + f64::from(a.width) / 2.,
            f64::from(a.y) + f64::from(a.height) / 2.,
        ],
        size: [a.width - 16, a.height - 16],
        angle: 0.13,
    });
    let mut recipe = project.image_recipe(id).unwrap();
    recipe.neutral.correction = scan.flat_field().cloned();
    let input = recipe.neutral.processing_input().unwrap();
    let scalar =
        SpatialPipeline::new(input, recipe.roll, recipe.grade.clone(), scan.camera_to_ap1).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("captured.tif");
    let read = |path: &Path| {
        let mut decoder = Decoder::new(std::fs::File::open(path).unwrap()).unwrap();
        let DecodingResult::U16(pixels) = decoder.read_image().unwrap() else {
            panic!("expected RGB16")
        };
        pixels
    };
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let (result, pixels) = crate::export_timing::pixels::capture(|| {
            write(
                &renderer,
                &device,
                (&queue, &gate),
                Job {
                    scan: &scan,
                    field: Some(gpu_field.clone()),
                    recipe: &recipe,
                    format: Format::Tiff(target),
                },
                (&path, Overwrite::Refuse),
                &mut Execution::default(),
                || Ok(()),
            )
        });
        result.unwrap();
        let pixels = pixels.unwrap().codes;
        assert!(
            pixels.iter().any(|pixel| pixel != &pixels[0]),
            "authored corrected ramp must retain rendered variation"
        );
        let expected_path = directory.path().join(format!("scalar-{target:?}.tif"));
        let clipped_path = directory.path().join(format!("clipped-{target:?}.tif"));
        for (path, clip) in [(&expected_path, false), (&clipped_path, true)] {
            crate::export::tiff_composed(
                path,
                recipe.composition,
                target,
                Overwrite::Refuse,
                ProcessingMetadata::with_grade(recipe.neutral.input.response(), &recipe.grade),
                |x, y| {
                    let pixel = scan.pixel(x, y)?;
                    scalar
                        .apply(if clip { pixel.min(DVec3::ONE) } else { pixel }, [x, y])
                        .map(|p| p.rgb)
                },
            )
            .unwrap();
        }
        let expected = read(&expected_path);
        let clipped = read(&clipped_path);
        assert_ne!(
            expected, clipped,
            "reference fixture must expose lost source headroom"
        );
        let actual: Vec<_> = pixels.into_iter().flatten().collect();
        assert_eq!(actual.len(), expected.len());
        let maximum = actual
            .iter()
            .zip(&expected)
            .map(|(&a, &b)| a.abs_diff(b))
            .max()
            .unwrap();
        println!("export_corrected_scalar target={target:?} maximum_rgb16={maximum}");
        assert!(
            maximum <= 1,
            "independent scalar/OCIO reference, target={target:?}"
        );
    }
}
