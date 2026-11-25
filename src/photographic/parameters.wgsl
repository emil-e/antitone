struct CameraMatrix {
    columns: array<vec4f, 3>,
}
@group(0) @binding(3)
var<uniform> camera_matrix: CameraMatrix;

struct Parameters {
    dmin: vec4f,
    dmax: vec4f,
    gamma_mode: vec4f,
    anchor: vec4f,
    post: array<vec4f, 3>,
    shape: vec4f,
    tail: array<vec4f, 3>,
    stage: u32,
}
@group(0) @binding(2)
var<uniform> parameters: Parameters;
