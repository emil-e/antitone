struct GradingContext {
    gain: vec4f,
    coverage: vec4u,
}
@group(0) @binding(8)
var<uniform> grading_context: GradingContext;
