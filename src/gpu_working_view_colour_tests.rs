//! Production working-view model, mapped masks and encoded-resource lifetime checks.
use super::*;
use crate::adjustment_layers::AdjustmentLayerSettings;
#[path = "gpu_working_view_model_tests.rs"]
mod model;
#[path = "gpu_working_view_oracle_tests.rs"]
mod oracles;
use crate::{
    adjustment_layers::{AdjustmentLayer, AdjustmentLayerCoverage},
    geometry::{FramePoint, SourcePoint, SourceRegion},
};

pub(super) fn authored_source(
    device: &wgpu::Device,
    renderer: Arc<crate::renderer::GpuRenderer>,
    kernels: Arc<ViewKernels>,
    values: &[[f32; 4]],
    frame: FrameRaster,
) -> Arc<ViewSource> {
    let matrix = DMat3::from_cols(
        DVec3::new(0.6062, 0.0161, -0.0031),
        DVec3::new(0.3802, 1.1927, -0.1490),
        DVec3::new(0.0136, -0.2088, 1.1521),
    );
    let spec = ViewSourceSpec::new(1, frame, negative(0.65), matrix, None).unwrap();
    Arc::new(ViewSource {
        spec,
        renderer,
        kernels,
        pixels: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("identical signed/HDR acquired AP1, including provenance"),
            contents: bytemuck::cast_slice(values),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        }),
    })
}

fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: Arc<ViewSource>,
    snapshot: AdjustmentLayerSnapshot,
    target: OutputTarget,
) -> Arc<RenderedView> {
    render_with_basis(
        device,
        queue,
        source,
        snapshot,
        target,
        negative(0.65),
        RollSettings::default(),
    )
}

fn render_with_basis(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: Arc<ViewSource>,
    snapshot: AdjustmentLayerSnapshot,
    target: OutputTarget,
    input: impl Into<InputSettings>,
    roll: RollSettings,
) -> Arc<RenderedView> {
    let settings = input.into();
    let grade = Arc::new(
        GpuAdjustmentLayers::new(
            device,
            settings,
            roll,
            snapshot.clone(),
            source.spec.matrix,
            source.spec.area,
        )
        .unwrap(),
    );
    let input = ProcessingInput::from(grade);
    let view = RenderedView::new(
        device, source, settings, roll, snapshot, &input, target, None,
    )
    .unwrap();
    view.ensure_submitted(
        device,
        queue,
        &SubmissionGate::default(),
        &egui::Context::default(),
    );
    view
}

fn photo() -> AdjustmentLayerSettings {
    AdjustmentLayerSettings {
        exposure: 0.25,
        wb: Balance {
            warmth: 0.005,
            tint: -0.003,
        },
        contrast: 1.01,
        contrast_colour_mix: 0.25,
        saturation: 1.02,
        saturation_density: 0.4,
        ..Default::default()
    }
}

fn local_pair(area: SourceRegion) -> Vec<AdjustmentLayer> {
    let x = area.x as f64;
    let y = area.y as f64;
    let w = area.width as f64;
    let h = area.height as f64;
    vec![
        AdjustmentLayer {
            mask: AdjustmentLayerCoverage::Linear {
                start: SourcePoint([x + w * 0.15, y + h * 0.2]),
                end: SourcePoint([x + w * 0.85, y + h * 0.8]),
            },
            photo: photo(),
            enabled: true,
            invert: true,
            strength: 0.6,
        },
        AdjustmentLayer {
            mask: AdjustmentLayerCoverage::Radial {
                center: SourcePoint([x + w * 0.5, y + h * 0.5]),
                radii: [w * 0.4, h * 0.4],
                angle: 0.3,
                feather: 0.8,
            },
            photo: AdjustmentLayerSettings {
                exposure: 0.3125,
                ..photo()
            },
            enabled: true,
            invert: false,
            strength: 0.6,
        },
    ]
}
