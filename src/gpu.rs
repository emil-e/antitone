//! GPU device setup and the modules that own calculations, resources and presentation.

#[cfg(any(feature = "render", feature = "project"))]
pub(crate) mod aces;
pub(crate) mod allocation;
#[cfg(feature = "editor")]
pub(crate) mod analysis;
#[cfg(feature = "raw")]
pub mod demosaic;
#[cfg(feature = "render")]
pub mod drt;
pub mod evaluation;
#[cfg(all(feature = "render", any(feature = "raw", feature = "ui")))]
pub(crate) mod execution;
pub mod flat;
pub mod grading;
pub mod interpretation;
pub mod lut3d;
pub mod photograph;
#[cfg(all(
    feature = "raw",
    feature = "render",
    feature = "project",
    any(feature = "editor", feature = "export")
))]
pub(crate) mod repair_source;
#[cfg(all(feature = "raw", feature = "render"))]
pub(crate) mod rgb16;
#[cfg(all(test, feature = "render"))]
pub(crate) mod test_support;

pub(crate) mod grain;
#[cfg(test)]
pub(crate) mod image_mtf;
pub(crate) mod raster;
#[cfg(any(
    feature = "editor",
    all(feature = "ui", feature = "render"),
    all(feature = "raw", feature = "render")
))]
pub(crate) mod recorded_image;

pub mod parameters;
pub(crate) mod repairs;
pub mod source;
pub mod submission;
#[cfg(test)]
pub(crate) mod test_inputs;
#[cfg(all(test, target_os = "macos", feature = "raw", feature = "export"))]
pub(crate) mod timestamp_ledger;
#[cfg(feature = "editor")]
pub(crate) mod view;

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

#[cfg(test)]
mod flat_tests;
#[cfg(test)]
mod input_grading_tests;
#[cfg(test)]
mod tests;
