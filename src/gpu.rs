//! Shared GPU device, allocation and submission infrastructure.

pub(crate) mod allocation;
pub mod submission;
#[cfg(all(test, target_os = "macos", feature = "raw", feature = "export"))]
pub(crate) mod timestamp_ledger;

/// The graphics API this platform presents and computes with, from the one `sys` selection.
pub fn backends() -> wgpu::Backends {
    crate::sys::backends()
}

/// An instance with this platform's graphics API and shader-compiler policy.
pub fn instance() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: crate::sys::backends(),
        backend_options: crate::sys::backend_options(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    })
}

#[cfg(feature = "render")]
pub(crate) const FULLSCREEN_VERTEX: &str = r#"
struct Vertex {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f
}

@vertex
fn main(@builtin(vertex_index) i: u32) -> Vertex {
    let p = array<vec2f, 3>(vec2f(-1., -1.), vec2f(3., -1.), vec2f(-1., 3.))[i];
    return Vertex(vec4f(p, 0., 1.), vec2f(p.x * .5 + .5, .5 - p.y * .5));
}"#;

#[cfg(all(feature = "render", any(feature = "ui", test)))]
pub(crate) fn fullscreen_pipeline(
    device: &wgpu::Device,
    vertex: &wgpu::ShaderModule,
    fragment: &wgpu::ShaderModule,
    layout: Option<&wgpu::BindGroupLayout>,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let layout = layout.map(|layout| {
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(layout)],
            immediate_size: 0,
        })
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("image render pipeline"),
        layout: layout.as_ref(),
        vertex: wgpu::VertexState {
            module: vertex,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: fragment,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    })
}

// Compatibility paths; these features own their implementations.
pub use crate::acquisition::camera as source;
pub use crate::evaluation;
pub use crate::gaussian::operation as gaussian;
pub use crate::photographic::parameters;
pub use crate::photographic::stages::flat_field as flat;
pub use crate::photographic::stages::{grading, interpretation};

// Preserve public paths for device-scoped Look and display resources.
#[cfg(feature = "render")]
pub use crate::photographic::stages::display_rendering as drt;
pub use crate::photographic::stages::look as lut3d;
