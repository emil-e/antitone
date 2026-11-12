use super::*;
use antitone::{
    adjustment_layers::AdjustmentLayerSnapshot,
    export::{ExrPrecision, Overwrite},
    geometry::Composition,
    gpu::submission::SubmissionGate,
    gpu_export::Format,
    renderer::GpuRenderer,
};
use authored_export::Request;

#[test]
fn repaired_source_supplies_measurements_proxies_and_scene_linear_export() {
    let mut fixture = Fixture {
        signal: [3686, 4096, 4096],
        neutral: [(1, 2), (1, 1), (2, 3)],
        ..Default::default()
    };
    fixture.lists[2].push(Opcode::warp(&[[0.98, 0.015, 0., 0., 0., 0.]], [0.5, 0.5]));
    let file = fixture.write();
    let mut scan = Scan::open(file.path(), None).unwrap();
    let mean = (2. * 3686. / 4096. + 1. + 1.5) / 3.;
    let expected = DVec3::splat(mean);
    let measured = scan.reference(Rect::from([12, 12, 3, 3])).unwrap();
    assert!((measured - expected).abs().max_element() < 2e-6);
    let (_, _, proxy) = scan.preview_source(8).unwrap();
    assert!(!proxy.is_empty());
    for pixel in proxy {
        let actual = DVec3::new(pixel[0].into(), pixel[1].into(), pixel[2].into()) / 65535.;
        assert!(
            (actual - expected).abs().max_element() < 2e-6,
            "proxy must sample the same repaired originals"
        );
    }
    let (device, queue, renderer) = pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let renderer = GpuRenderer::new(&device, &queue).await.unwrap();
        (device, queue, renderer)
    });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("highlight.exr");
    authored_export::write(
        &renderer,
        &device,
        (&queue, &SubmissionGate::default()),
        Request {
            scan: &scan,
            field: None,
            input: InputSettings::Digital,
            roll: Default::default(),
            grade: AdjustmentLayerSnapshot::new(vec![]).unwrap(),
            repairs: &[],
            composition: Composition::new(scan.area, Default::default(), Default::default())
                .unwrap(),
            format: Format::Exr(ExrPrecision::Full),
            luts: Default::default(),
            pipeline: Default::default(),
        },
        &path,
        Overwrite::Refuse,
    )
    .unwrap();
    let image = exr::prelude::read_first_flat_layer_from_file(path).unwrap();
    let expected = antitone::color::ap1_to_ap0() * scan.camera_to_ap1 * expected;
    assert_eq!(image.layer_data.channel_data.list.len(), 3);
    for channel in &image.layer_data.channel_data.list {
        let c = match channel.name.to_string().as_str() {
            "R" => 0,
            "G" => 1,
            "B" => 2,
            _ => panic!("unexpected channel"),
        };
        let exr::prelude::FlatSamples::F32(values) = &channel.sample_data else {
            panic!("full f32 EXR expected")
        };
        assert!(!values.is_empty());
        for &value in values {
            assert!(
                (f64::from(value) - expected[c]).abs() < 6e-6,
                "GPU export must retain the analytic repaired camera colour"
            );
        }
    }
}
