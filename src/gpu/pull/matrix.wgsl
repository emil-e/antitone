struct PullMatrix {
    columns: array<vec4f, 3>,
}
@group(0) @binding(18)
var<uniform> pull_matrix: PullMatrix;
fn pull_interpret(pixel: vec4f, position: vec2f) -> vec4f {
    let rgb = pull_matrix.columns[0].xyz * pixel.r + pull_matrix.columns[1].xyz * pixel.g
        + pull_matrix.columns[2].xyz * pixel.b;
    return vec4f(rgb, pixel.w);
}
