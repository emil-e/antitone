//! Callable image sampling: virtual images can cross operations without a buffer per layer.
use super::*;
use crate::geometry::SourceRegion as Rect;

pub enum SampleInput {
    Raster(PlannedImage),
    Callable(Sampled),
}
pub enum Access {
    Raster,
    Callable,
}
pub trait SampleProducer: Send + Sync {
    fn access(&self) -> Access;
    fn emit(
        &self,
        fragment: &mut Fragment,
        input: SampleInput,
        domain: Domain,
        region: Rect,
    ) -> Result<Sampled, Error>;
}
pub trait SampleConsumer: Send + Sync {
    fn emit(
        &self,
        fragment: &mut Fragment,
        input: Sampled,
        output: PlannedImage,
    ) -> Result<(), Error>;
}

/// ABI: fn(vec2i) -> vec4f, addressed on the declared input grid. Code/resources are
/// explicit; the shader's mathematics remains opaque to planning and execution.
pub struct Sampled {
    pub code: Arc<str>,
    pub symbol: String,
    pub bindings: Vec<(u32, Binding)>,
}
impl Sampled {
    pub fn raster(fragment: &mut Fragment, image: PlannedImage) -> Self {
        let parameters = fragment.uniform(&[
            image.region.x,
            image.region.y,
            image.region.width,
            image.region.height,
        ]);
        Self {
            code: r#"
@group(0) @binding(32) var<storage, read> sample_pixels: array<vec4f>;
@group(0) @binding(33) var<uniform> sample_rectangle: vec4u;
fn sample_buffer(p: vec2i) -> vec4f {
    let local = p - vec2i(sample_rectangle.xy);
    return sample_pixels[u32(local.y) * sample_rectangle.z + u32(local.x)];
}
"#
            .into(),
            symbol: "sample_buffer".into(),
            bindings: vec![(32, Binding::Read(image.resource)), (33, parameters)],
        }
    }

    pub fn materialize(self, fragment: &mut Fragment, output: PlannedImage) -> Result<(), Error> {
        let parameters = fragment.uniform(&[
            output.region.x,
            output.region.y,
            output.region.width,
            output.region.height,
        ]);
        let call = format!("{}(vec2i(p))", self.symbol);
        let code = format!(
            r#"
{}
@group(0) @binding(21) var<storage, read_write> sample_result: array<vec4f>;
@group(0) @binding(22) var<uniform> sample_output: vec4u;
@compute @workgroup_size(64)
fn sample_main(@builtin(global_invocation_id) id: vec3u, @builtin(num_workgroups) groups: vec3u) {{
    let index = id.x + id.y * groups.x * 64u;
    if index >= sample_output.z * sample_output.w {{ return; }}
    let p = sample_output.xy + vec2u(index % sample_output.z,index / sample_output.z);
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
            // Transcode the same adapted pinned functions, not a hand-written ACES approximation.
            let source = format!(
                "#version 450\n{}\n{code}\nvoid main() {{}}",
                "layout(set=0,binding=3) uniform sampler aces_sampler;"
            );
            let options = naga::front::glsl::Options::from(naga::ShaderStage::Compute);
            let mut module = naga::front::glsl::Frontend::default()
                .parse(&options, &source)
                .map_err(|e| Error::Gpu(format!("ACES callable GLSL: {e:?}")))?;
            module.entry_points.clear();
            for (_, function) in module.functions.iter_mut() {
                if function.name.as_deref() == Some("main") {
                    function.name = Some("aces_library_entry_stub".into());
                }
                // The pinned table textures have one mip level. Explicit level zero gives the
                // same lookup in compute without fragment-only implicit derivatives.
                for (_, expression) in function.expressions.iter_mut() {
                    if let naga::Expression::ImageSample { level, .. } = expression
                        && matches!(level, naga::SampleLevel::Auto)
                    {
                        *level = naga::SampleLevel::Zero;
                    }
                }
            }
            let mut validator = naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            );
            let info = validator
                .validate(&module)
                .map_err(|e| Error::Gpu(format!("ACES callable validation: {e:?}")))?;
            naga::back::wgsl::write_string(
                &module,
                &info,
                naga::back::wgsl::WriterFlags::EXPLICIT_TYPES,
            )
            .map(Arc::from)
            .map_err(|e| Error::Gpu(format!("ACES callable WGSL: {e}")))
        })
        .clone()
}
