use super::*;
use crate::{
    adjustment_layers::AdjustmentLayerSnapshot,
    crop::Crop,
    export::Overwrite,
    geometry::Composition,
    gpu::SubmissionGate,
    gpu_export::{self, Format, Request},
    processing::{Calibration, RollSettings},
    renderer::GpuRenderer,
    rendering::OutputTarget,
    scan::Scan,
};
use glam::DVec3;
use std::path::Path;

#[test]
fn deferred_counters_preserve_common_pixels_and_reject_final_collection_before_publication() {
    let (device, queue, renderer) = pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_features: wgpu::Features::TIMESTAMP_QUERY,
                ..Default::default()
            })
            .await
            .unwrap();
        let renderer = GpuRenderer::new(&device, &queue).await.unwrap();
        (device, queue, renderer)
    });
    let scan = Scan::open(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        None,
    )
    .unwrap();
    let a = scan.area;
    // One destination unit leaves its only query collection deferred until finalization.
    let composition = Composition::with_crop(
        a,
        Default::default(),
        Default::default(),
        Some(Crop {
            center: [
                f64::from(a.x) + f64::from(a.width) / 2.,
                f64::from(a.y) + f64::from(a.height) / 2.,
            ],
            size: [32; 2],
            angle: 0.13,
        }),
    )
    .unwrap();
    let request = || Request {
        pipeline: Default::default(),
        scan: &scan,
        field: None,
        input: Calibration {
            dmin: DVec3::splat(0.8),
            dmax: DVec3::splat(0.02),
            input_floor: scan.input_floor,
        }
        .into(),
        roll: RollSettings::default(),
        grade: AdjustmentLayerSnapshot::new(vec![
            crate::adjustment_layers::AdjustmentLayer::global(Default::default()),
        ])
        .unwrap(),
        repairs: &[],
        composition,
        format: Format::Tiff(OutputTarget::Srgb),
        look: None,
    };
    let gate = SubmissionGate::default();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("destination.tif");
    let capture = || {
        let (result, pixels) = crate::export_timing::pixels::capture(|| {
            gpu_export::write(
                &renderer,
                &device,
                (&queue, &gate),
                request(),
                &path,
                Overwrite::Refuse,
            )
        });
        result.unwrap();
        let pixels = pixels.unwrap();
        assert_eq!(pixels.codes.len(), 32 * 32);
        pixels.codes
    };
    let expected = capture();
    let clock = Timebase::calibrate(&device, &queue).unwrap();
    for _ in 0..2 {
        let actual = with_clock(Some(clock.clone()), capture);
        assert_eq!(actual, expected);
        clock.require_valid().unwrap();
    }
    std::fs::write(&path, b"existing destination").unwrap();
    let invalid = clock.invalid_for_collection_test();
    assert!(
        with_clock(Some(invalid.clone()), || gpu_export::write(
            &renderer,
            &device,
            (&queue, &gate),
            request(),
            &path,
            Overwrite::Confirmed,
        ))
        .is_err()
    );
    assert!(
        invalid.require_valid().is_err(),
        "failure must come from collection, not an unexercised setup branch"
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"existing destination");
    assert_eq!(
        std::fs::read_dir(directory.path()).unwrap().count(),
        1,
        "failed collection must retire the temporary file"
    );
}
