struct EvaluationMatrix {
    columns: array<vec4f, 3>,
}
@group(0) @binding(18)
var<uniform> evaluation_matrix: EvaluationMatrix;
fn evaluation_interpret(pixel: vec4f, position: vec2f) -> vec4f {
    let rgb =
        evaluation_matrix.columns[0].xyz * pixel.r + evaluation_matrix.columns[1].xyz * pixel.g
        + evaluation_matrix.columns[2].xyz * pixel.b;
    return vec4f(rgb, pixel.w);
}
