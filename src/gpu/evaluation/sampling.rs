//! Callable image sampling: virtual images can cross operations without a buffer per node.

use super::CellRegion as Rect;
use super::*;

mod library;
mod linkage;
mod parsed;
#[cfg(all(test, feature = "editor"))]
pub(super) use parsed::testing as library_testing;
pub(super) use parsed::{Libraries, Library};

#[derive(Clone)]
pub(crate) enum SampleInput {
    Raster(PlannedImage),
    Callable(Sampled),
}

pub(crate) enum Access {
    Raster,
    Callable,
}

/// Image samplers return vec4f on their declared grid; the value compiler also uses
/// typed unit-local records. Code/resources are explicit; mathematics stays opaque.
#[derive(Clone)]
pub(crate) struct Sampled {
    pub code: Arc<str>,
    pub symbol: String,
    pub bindings: Vec<(u32, Binding)>,
}

impl Sampled {
    pub fn raster(fragment: &mut Fragment, image: PlannedImage) -> Self {
        Self::buffer(fragment, image.region, Binding::Read(image.resource))
    }

    pub(super) fn buffer(fragment: &mut Fragment, region: Rect, pixels: Binding) -> Self {
        Self::buffer_type(fragment, region, pixels, "vec4f")
    }

    pub(super) fn buffer_type(
        fragment: &mut Fragment,
        region: Rect,
        pixels: Binding,
        ty: &str,
    ) -> Self {
        let parameters = fragment.uniform(&region.packed());
        Self {
            code: r#"
@group(0) @binding(32)
var<storage, read> sample_pixels: array<vec4f>;

@group(0) @binding(33)
var<uniform> sample_rectangle: vec4u;

fn sample_buffer(p: vec2i) -> vec4f {
    let local = p - vec2i(sample_rectangle.xy);
    return sample_pixels[u32(local.y) * sample_rectangle.z + u32(local.x)];
}
"#
            .replace("vec4f", ty)
            .into(),
            symbol: "sample_buffer".into(),
            bindings: vec![(32, pixels), (33, parameters)],
        }
    }

    #[cfg(feature = "render")]
    pub(super) fn render(self, fragment: &mut Fragment, output: PlannedImage) -> Result<(), Error> {
        let mut slot = 64;
        let sample = self.namespaced(fragment, "attachment_", &mut slot)?;
        let region = fragment.uniform(&[
            output.region.packed(),
            [u32::from(output.domain.grid.coverage != 0), 0, 0, 0],
        ]);
        let code = format!(
            r#"
{}
{}
@group(0) @binding({slot}) var<uniform> attachment_region:array<vec4u,2>;
@fragment fn present_main(@builtin(position) position:vec4f)->@location(0) vec4f {{
    let cell=bitcast<vec2i>(attachment_region[0].xy)+vec2i(position.xy);
    let value={}(cell);
    if value.w<0.0 {{return value;}}
    if attachment_region[1].x!=0u && (u32(value.w)&8u)==0u {{return vec4f(0.0);}}
    return vec4f(value.rgb,1.0);
}}
"#,
            sample.code,
            crate::renderer::VERTEX,
            sample.symbol
        );
        let mut bindings = sample.bindings;
        bindings.push((slot, region));
        fragment.work.push(Work::Render {
            program: RenderProgram::Shader(code.into()),
            bindings,
            target: output.resource,
        });
        Ok(())
    }

    pub fn materialize(self, fragment: &mut Fragment, output: PlannedImage) -> Result<(), Error> {
        self.materialize_type(fragment, output, "vec4f")
    }

    pub(super) fn materialize_type(
        self,
        fragment: &mut Fragment,
        output: PlannedImage,
        ty: &str,
    ) -> Result<(), Error> {
        let parameters = fragment.uniform(&output.region.packed());
        let call = format!("{}(vec2i(p))", self.symbol);
        let code = format!(
            r#"
{}
@group(0) @binding(21) var<storage, read_write> sample_result: array<{ty}>;
@group(0) @binding(22) var<uniform> sample_output: vec4u;

@compute @workgroup_size(64)
fn sample_main(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {{
    let index = id.x + id.y * groups.x * 64u;
    if index >= sample_output.z * sample_output.w {{
        return;
    }}

    let p = sample_output.xy + vec2u(index % sample_output.z, index / sample_output.z);
    sample_result[index] = {call};
}}
"#,
            self.code
        );
        let mut bindings = self.bindings;
        if bindings.iter().any(|(slot, _)| *slot == 21 || *slot == 22) {
            return Err(Error::InvalidParameter(
                "sample code collides with materialization ABI",
            ));
        }
        bindings.extend([(21, Binding::Write(output.resource)), (22, parameters)]);
        fragment.compute(code, "sample_main", bindings, output.cells());
        Ok(())
    }
}

#[cfg(feature = "render")]
pub fn display_code(target: crate::rendering::OutputTarget) -> Result<Arc<str>, Error> {
    use std::sync::OnceLock;

    static SRGB: OnceLock<Result<Arc<str>, Error>> = OnceLock::new();

    static P3: OnceLock<Result<Arc<str>, Error>> = OnceLock::new();
    let cache = match target {
        crate::rendering::OutputTarget::Srgb => &SRGB,
        crate::rendering::OutputTarget::DisplayP3 => &P3,
    };
    cache
        .get_or_init(|| {
            let code = crate::renderer::target_function_glsl(target)?;
            crate::gpu::aces::library(&code)
        })
        .clone()
}
