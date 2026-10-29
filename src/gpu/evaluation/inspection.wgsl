struct Inspection {
    columns: array<vec4f, 3>,
    coverage: vec4u,
}
@group(0) @binding(0)
var<uniform> inspection: Inspection;

fn inspect_source(input: vec4f, point: vec2f) -> vec4f {
    if inspection.coverage.x != 0u && (u32(input.w) & inspection.coverage.x) == 0u {
        return vec4f(0.);
    }
    let transform = mat3x3f(
            inspection.columns[0].xyz,
            inspection.columns[1].xyz,
            inspection.columns[2].xyz,
        );
    return vec4f(transform * input.xyz, input.w);
}
