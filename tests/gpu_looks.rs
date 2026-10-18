#![cfg(all(
    feature = "gpu",
    feature = "render",
    feature = "project",
    target_os = "macos"
))]
use antitone::{
    gpu::SubmissionGate,
    looks::builtin::{Builtin, DEFAULT_GRID, DOMAIN_MIN, Model, domain_max},
    renderer::GpuRenderer,
};
use glam::DVec3;
use std::sync::atomic::AtomicBool;

fn decode(value: f64) -> f64 {
    if value <= 0.155251141552511 {
        (value - 0.0729055341958355) / 10.5402377416545
    } else {
        (17.52 * value - 9.72).exp2()
    }
}

fn grid_input(index: usize, size: u32) -> DVec3 {
    let size = size as usize;
    DVec3::from_array(
        [index % size, (index / size) % size, index / (size * size)].map(|coordinate| {
            decode(DOMAIN_MIN + (domain_max() - DOMAIN_MIN) * coordinate as f64 / (size - 1) as f64)
        }),
    )
}

#[test]
fn default_grid_generation_keeps_reference_gray_device_and_project_admission() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let gate = SubmissionGate::default();
        let renderer = GpuRenderer::new_with_submissions(&device, &queue, gate.clone())
            .await
            .unwrap();
        let directory = tempfile::tempdir().unwrap();

        for builtin in Builtin::ALL {
            let asset = std::sync::Arc::new(
                renderer
                    .bake_builtin(
                        &device,
                        &queue,
                        &gate,
                        builtin,
                        DEFAULT_GRID,
                        &AtomicBool::new(false),
                    )
                    .unwrap(),
            );
            assert_eq!(asset.lut().size(), DEFAULT_GRID);
            assert!(
                (asset.lut().apply(DVec3::splat(0.18)).unwrap() - DVec3::splat(0.18))
                    .abs()
                    .max_element()
                    < 2e-5
            );
            let resource = antitone::gpu::look::Lut::new(&device, asset.lut()).unwrap();
            assert_eq!(resource.byte_len(), (DEFAULT_GRID as u64).pow(3) * 16);

            let project = antitone::project::Project {
                output: antitone::project::Output {
                    look: Some(asset.clone()),
                    ..Default::default()
                },
                ..Default::default()
            };
            let path = directory.path().join(format!("{}.antitone", builtin.id()));
            project.save(&path, antitone::project::Save::New).unwrap();
            let reopened = antitone::project::Project::load(&path).unwrap();
            assert_eq!(reopened.output.look, Some(asset.clone()));

            #[cfg(feature = "ocio-reference")]
            {
                let model = Model::new(builtin).unwrap();
                let cpu = antitone::rendering::CpuRenderer::new_reference(
                    antitone::looks::builtin::REFERENCE,
                )
                .unwrap();
                let scene = asset
                    .lut()
                    .nodes()
                    .iter()
                    .map(|node| DVec3::from_array(node.map(|value| decode(value as f64))))
                    .collect::<Vec<_>>();
                let actual = cpu.render(&scene).unwrap();
                let mut maximum = 0f64;
                for (index, actual) in actual.iter().enumerate() {
                    maximum = maximum.max(
                        (actual.as_dvec3()
                            - model.mapped(grid_input(index, DEFAULT_GRID)).unwrap())
                        .abs()
                        .max_element(),
                    );
                }
                println!(
                    "{} default {DEFAULT_GRID}³ all-node independent fixed-reference error={maximum}",
                    builtin.id()
                );
                assert!(
                    maximum <= 2e-4,
                    "{} default reference admission failed: {maximum}",
                    builtin.id()
                );
            }
        }
    });
}

#[test]
fn native_generation_matches_the_frozen_65_grid_material_reference() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let gate = SubmissionGate::default();
        let renderer = GpuRenderer::new_with_submissions(&device, &queue, gate.clone())
            .await
            .unwrap();
        let reference: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/looks/native-65.json")).unwrap();
        assert_eq!(reference["schema"], 1);
        assert_eq!(reference["size"], 65);
        // This independent fixture stays at its authored grid, not the application default.
        let input_at = |index| grid_input(index, 65);
        let mut ids = Vec::new();
        for builtin in Builtin::ALL {
            let asset = renderer
                .bake_builtin(&device, &queue, &gate, builtin, 65, &AtomicBool::new(false))
                .unwrap();
            assert_eq!(asset.lut().size(), 65);
            assert_eq!(asset.lut().nodes().len(), 65usize.pow(3));
            assert!(
                (asset.lut().apply(DVec3::splat(0.18)).unwrap() - DVec3::splat(0.18))
                    .abs()
                    .max_element()
                    < 2e-5
            );
            let native = &reference["looks"][builtin.id()];
            let manifest: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("assets/looks")
                        .join(builtin.id())
                        .join("look.json"),
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(asset.name(), manifest["name"].as_str().unwrap());
            let stored = serde_json::to_value(&asset).unwrap();
            assert_eq!(stored["name"], manifest["name"]);
            assert_eq!(native["model_sha256"], manifest["model_sha256"]);
            assert_eq!(
                native["numeric_payload_sha256"],
                manifest["reviewed_bakes"]["65"]["numeric_payload_sha256"]
            );
            assert_eq!(native["samples"].as_array().unwrap().len(), 256);
            let model = Model::new(builtin).unwrap();
            let mut maximum_scene = 0f64;
            let mut maximum_material = 0f64;
            for sample in native["samples"].as_array().unwrap() {
                assert_eq!(sample.as_array().unwrap().len(), 7);
                let index = sample[0].as_u64().unwrap() as usize;
                let mapped = model.mapped(input_at(index)).unwrap();
                for c in 0..3 {
                    maximum_material =
                        maximum_material.max((mapped[c] - sample[c + 4].as_f64().unwrap()).abs());
                    maximum_scene = maximum_scene.max(
                        (decode(asset.lut().nodes()[index][c] as f64)
                            - decode(sample[c + 1].as_f64().unwrap()))
                        .abs(),
                    );
                }
            }
            assert!(
                maximum_material <= 1e-9,
                "{} native material port error {maximum_material}",
                builtin.id()
            );
            println!(
                "{} independent native samples: f64 material error={maximum_material}; scene AP1 CPU/GPU inverse discrepancy={maximum_scene}",
                builtin.id()
            );
            // The established 2e-4 gate is fixed-reference RGB, not scene AP1 units.
            // Independently check it with OCIO CPU, not only the paired GPU shaders.
            #[cfg(feature = "ocio-reference")]
            {
                let cpu = antitone::rendering::CpuRenderer::new_reference(
                    antitone::looks::builtin::REFERENCE,
                )
                .unwrap();
                let scene = asset
                    .lut()
                    .nodes()
                    .iter()
                    .map(|node| DVec3::from_array(node.map(|value| decode(value as f64))))
                    .collect::<Vec<_>>();
                let actual = cpu.render(&scene).unwrap();
                let mut maximum = 0f64;
                for (index, actual) in actual.iter().enumerate() {
                    maximum = maximum.max(
                        (actual.as_dvec3() - model.mapped(input_at(index)).unwrap())
                            .abs()
                            .max_element(),
                    );
                }
                println!(
                    "{} all-node independent OCIO CPU fixed-reference error={maximum}",
                    builtin.id()
                );
                assert!(
                    maximum <= 2e-4,
                    "{} independent reference admission failed: {maximum}",
                    builtin.id()
                );
            }
            assert_eq!(asset.origin(), &builtin.origin());
            assert!(!ids.contains(&asset.lut().id()));
            ids.push(asset.lut().id());
        }
        assert!(
            renderer
                .bake_builtin(
                    &device,
                    &queue,
                    &gate,
                    Builtin::PortraEndura,
                    65,
                    &AtomicBool::new(true)
                )
                .is_err()
        );
        assert!(
            renderer
                .bake_builtin(
                    &device,
                    &queue,
                    &gate,
                    Builtin::PortraEndura,
                    17,
                    &AtomicBool::new(false)
                )
                .is_err()
        );
    });
}
